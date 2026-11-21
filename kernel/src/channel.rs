use channel_state::{BYTES, HANDLES, Hop, Inbox, Link, Route, Span, route};
use native_abi::Rights;
use native_abi::limits::{MESSAGE_BYTES, MESSAGE_HANDLES};
use wost::mm::pool::{ArenaFrame, Earmark, FrameKind};
use wost::mm::slab::{Hold, Refusal, Released, Removal};
use wost::sync::SpinLock;

use crate::arena::Local;
use crate::object::ObjectRef;
use crate::queue::QueueRef;
use crate::rank;
use crate::world::{EndpointPool, World};

const _: () = assert!(
    MESSAGE_BYTES <= BYTES as u32 && MESSAGE_HANDLES <= HANDLES as u32,
    "an inbox holds at least one message of every size the ABI admits"
);

pub(crate) const RIGHTS: Rights = Rights::DUPLICATE
    .union(Rights::TRANSFER)
    .union(Rights::READ)
    .union(Rights::WRITE);

// Reads waiting on one endpoint; a write wakes the oldest.
const ARMED: usize = 8;

pub(crate) type Moved = [Option<(ObjectRef, Rights)>; MESSAGE_HANDLES as usize];

pub(crate) const NONE_MOVED: Moved = [None; MESSAGE_HANDLES as usize];

// A pending ring read, named by its queue, slot and never-reused id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Reader {
    pub(crate) queue: QueueRef,
    pub(crate) slot: usize,
    pub(crate) id: u64,
}

// The inbox is paid for by its holder's world when the endpoint is made: the record, and one
// frame for the byte ring.
#[derive(Debug)]
pub(crate) struct Endpoint {
    state: SpinLock<rank::Object, State>,
}

#[derive(Debug)]
struct State {
    link: Link<Local<Endpoint>>,
    inbox: Inbox,
    // A moved object's strong reference, held here until a reader installs it.
    transit: [Option<(ObjectRef, Rights)>; HANDLES as usize],
    bytes: ArenaFrame,
    armed: [Option<Reader>; ARMED],
    doomed: Option<Local<Endpoint>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WriteRefusal {
    PeerClosed,
    Full,
    // An endpoint written into its own inbox.
    Denied,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Head {
    Empty,
    PeerClosed,
    Message { bytes: u16, handles: u16 },
}

impl Endpoint {
    pub(crate) fn frames_for(endpoints: u32) -> Option<u32> {
        EndpointPool::frames_for(endpoints)?.checked_add(endpoints)
    }

    fn make(earmark: &mut Earmark, link: Link<Local<Self>>) -> Option<Self> {
        Some(Self {
            state: SpinLock::new(State {
                link,
                inbox: Inbox::EMPTY,
                transit: [None; HANDLES as usize],
                bytes: earmark.take(FrameKind::ArenaData)?,
                armed: [None; ARMED],
                doomed: None,
            }),
        })
    }
}

// Two endpoints in one world, each naming the other.
pub(crate) fn create(
    earmark: &mut Earmark,
    world: &World,
) -> Option<(Local<Endpoint>, Local<Endpoint>)> {
    let first = Endpoint::make(earmark, Link::PeerClosed)?;
    let first = world.endpoints.insert(earmark, first);
    let Some(second) = Endpoint::make(earmark, Link::Open(first)) else {
        release(world, first);
        return None;
    };
    let second = world.endpoints.insert(earmark, second);
    let Some(held) = world.endpoints.hold(first) else {
        panic!("a fresh endpoint went before its peer named it");
    };
    held.state.lock_leaf().link = Link::Open(second);
    Some((first, second))
}

// The birth endpoint of a program: nothing is at the other end.
pub(crate) fn alone(earmark: &mut Earmark, world: &World) -> Option<Local<Endpoint>> {
    let endpoint = Endpoint::make(earmark, Link::PeerClosed)?;
    Some(world.endpoints.insert(earmark, endpoint))
}

fn held(world: &World, endpoint: Local<Endpoint>) -> Hold<Endpoint> {
    world
        .endpoints
        .hold(endpoint)
        .unwrap_or_else(|| panic!("an endpoint went while a strong reference held it"))
}

// Commits one message into the peer's inbox and hands back the oldest armed reader to wake.
// On a refusal the moved objects are still the caller's.
pub(crate) fn write(
    world: &World,
    from: Local<Endpoint>,
    bytes: &[u8],
    moved: &mut Moved,
    count: usize,
) -> Result<Option<Reader>, WriteRefusal> {
    let Link::Open(to) = held(world, from).state.lock_leaf().link else {
        return Err(WriteRefusal::PeerClosed);
    };
    // The peer is reached only under a reference of its own, so it cannot go mid-write.
    if world.endpoints.acquire(to).is_err() {
        return Err(WriteRefusal::PeerClosed);
    }
    let committed = if moved
        .iter()
        .flatten()
        .any(|(object, _)| *object == ObjectRef::Endpoint(to))
    {
        Err(WriteRefusal::Denied)
    } else {
        commit(&held(world, to), from, bytes, moved, count)
    };
    release(world, to);
    committed
}

fn commit(
    to: &Endpoint,
    from: Local<Endpoint>,
    bytes: &[u8],
    moved: &mut Moved,
    count: usize,
) -> Result<Option<Reader>, WriteRefusal> {
    let mut state = to.state.lock_leaf();
    if route(from, state.link, Hop::First) != Route::Deliver {
        return Err(WriteRefusal::PeerClosed);
    }
    let (Ok(length), Ok(handles)) = (u16::try_from(bytes.len()), u16::try_from(count)) else {
        return Err(WriteRefusal::Full);
    };
    let entry = state
        .inbox
        .push(length, handles)
        .map_err(|_| WriteRefusal::Full)?;
    let mut done = 0;
    for span in entry.descriptor.byte_spans() {
        let chunk = &bytes[done..done + usize::from(span.len)];
        assert!(
            state.bytes.write_at(usize::from(span.at), chunk),
            "an inbox span left its byte ring"
        );
        done += chunk.len();
    }
    let mut taken = moved.iter_mut().map(Option::take);
    for slot in spanned(entry.descriptor.handle_spans()) {
        state.transit[slot] = taken.next().flatten();
    }
    Ok(pop_armed(&mut state))
}

fn spanned(spans: [Span; 2]) -> impl Iterator<Item = usize> {
    spans
        .into_iter()
        .flat_map(|span| usize::from(span.at)..usize::from(span.at) + usize::from(span.len))
}

fn pop_armed(state: &mut State) -> Option<Reader> {
    let first = state.armed[0].take();
    state.armed.rotate_left(1);
    first
}

pub(crate) fn head(world: &World, endpoint: Local<Endpoint>) -> Head {
    let hold = held(world, endpoint);
    let state = hold.state.lock_leaf();
    match state.inbox.head() {
        Some(entry) => Head::Message {
            bytes: entry.descriptor.bytes(),
            handles: entry.descriptor.handles(),
        },
        None if state.link == Link::PeerClosed => Head::PeerClosed,
        None => Head::Empty,
    }
}

// Pops the head message when it fits the room the reader holds; `None` leaves it queued.
pub(crate) fn take(
    world: &World,
    endpoint: Local<Endpoint>,
    room: (u16, u16),
    into: &mut [u8],
    moved: &mut Moved,
) -> Option<(u16, u16)> {
    let hold = held(world, endpoint);
    let mut state = hold.state.lock_leaf();
    let entry = state.inbox.head()?;
    let (bytes, handles) = (entry.descriptor.bytes(), entry.descriptor.handles());
    if bytes > room.0 || handles > room.1 {
        return None;
    }
    let mut done = 0;
    for span in entry.descriptor.byte_spans() {
        let chunk = &mut into[done..done + usize::from(span.len)];
        assert!(
            state.bytes.read_at(usize::from(span.at), chunk),
            "an inbox span left its byte ring"
        );
        done += chunk.len();
    }
    for (at, slot) in spanned(entry.descriptor.handle_spans()).enumerate() {
        moved[at] = state.transit[slot].take();
    }
    state.inbox.pop();
    Some((bytes, handles))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ArmRefusal {
    Readable,
    Full,
}

// Arms a reader unless a message or a closed peer is already there to read.
pub(crate) fn arm(
    world: &World,
    endpoint: Local<Endpoint>,
    reader: Reader,
) -> Result<(), ArmRefusal> {
    let hold = held(world, endpoint);
    let mut state = hold.state.lock_leaf();
    if state.inbox.count() > 0 || state.link == Link::PeerClosed {
        return Err(ArmRefusal::Readable);
    }
    let free = state
        .armed
        .iter_mut()
        .find(|slot| slot.is_none())
        .ok_or(ArmRefusal::Full)?;
    *free = Some(reader);
    Ok(())
}

pub(crate) fn disarm(world: &World, endpoint: Local<Endpoint>, reader: Reader) {
    if world.endpoints.acquire(endpoint).is_err() {
        return;
    }
    {
        let hold = held(world, endpoint);
        let mut state = hold.state.lock_leaf();
        if let Some(at) = state.armed.iter().position(|slot| *slot == Some(reader)) {
            state.armed[at] = None;
            state.armed[at..].rotate_left(1);
        }
    }
    release(world, endpoint);
}

pub(crate) fn release(world: &World, endpoint: Local<Endpoint>) {
    if let Ok(Released::NowZero) | Err(Refusal::Condemned) = world.endpoints.release(endpoint) {
        destroy(world, endpoint);
    }
}

// Iterative, since endpoints in transit can nest to any depth: each one that loses its last
// reference joins the doomed chain instead of being destroyed inside another's destruction.
fn destroy(world: &World, first: Local<Endpoint>) {
    let mut doomed = Some(first);
    while let Some(endpoint) = doomed {
        let (link, armed) = {
            let held = held(world, endpoint);
            doomed = held.state.lock_leaf().doomed.take();
            loop {
                let mut moved = NONE_MOVED;
                {
                    let mut state = held.state.lock_leaf();
                    let Some(entry) = state.inbox.pop() else {
                        break;
                    };
                    for (at, slot) in spanned(entry.descriptor.handle_spans()).enumerate() {
                        moved[at] = state.transit[slot].take();
                    }
                }
                for (object, _) in moved.into_iter().flatten() {
                    discard(world, object, &mut doomed);
                }
            }
            let mut state = held.state.lock_leaf();
            let armed = core::mem::replace(&mut state.armed, [None; ARMED]);
            (core::mem::replace(&mut state.link, Link::PeerClosed), armed)
        };
        wake(armed);
        if let Link::Open(peer) = link {
            close_peer(world, peer, endpoint, &mut doomed);
        }
        match world.endpoints.remove(endpoint) {
            Ok(_) | Err(Removal::Refused(Refusal::Stale)) => {}
            Err(_) => panic!("an endpoint lost its last reference while a hold borrowed it"),
        }
    }
}

fn discard(world: &World, object: ObjectRef, doomed: &mut Option<Local<Endpoint>>) {
    let ObjectRef::Endpoint(endpoint) = object else {
        object.release(world);
        return;
    };
    if let Ok(Released::NowZero) | Err(Refusal::Condemned) = world.endpoints.release(endpoint) {
        held(world, endpoint).state.lock_leaf().doomed = doomed.replace(endpoint);
    }
}

fn close_peer(
    world: &World,
    peer: Local<Endpoint>,
    closed: Local<Endpoint>,
    doomed: &mut Option<Local<Endpoint>>,
) {
    if world.endpoints.acquire(peer).is_err() {
        return;
    }
    let armed = {
        let hold = held(world, peer);
        let mut state = hold.state.lock_leaf();
        if state.link == Link::Open(closed) {
            state.link = Link::PeerClosed;
        }
        core::mem::replace(&mut state.armed, [None; ARMED])
    };
    wake(armed);
    discard(world, ObjectRef::Endpoint(peer), doomed);
}

fn wake(armed: [Option<Reader>; ARMED]) {
    for reader in armed.into_iter().flatten() {
        reader.queue.ready(reader);
    }
}

// A dying world's endpoints that only reach each other through their inboxes go here.
pub(crate) fn dissolve(world: &World) {
    while let Some(endpoint) = world.endpoints.any() {
        while let Ok(Released::Held) = world.endpoints.release(endpoint) {}
        destroy(world, endpoint);
    }
}
