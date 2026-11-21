use core::ops::Deref;
use core::sync::atomic::Ordering;

use channel_state::{BYTES, DESCRIPTORS, HANDLES, Hop, Inbox, Link, Route, Span, route};
use native_abi::Rights;
use native_abi::limits::{MESSAGE_BYTES, MESSAGE_HANDLES};
use world_tree::{Admission, NodeRef};
use wost::mm::pool::{ArenaFrame, Earmark, FrameKind};
use wost::mm::slab::{Hold, Refusal, Released, Removal};
use wost::sync::SpinLock;

use crate::arena::Local;
use crate::object::{ENCODED, ObjectRef, ObjectType};
use crate::queue::{self, CallCell, Caller, QueueRef};
use crate::rank;
use crate::world::registry::{self, Pin};
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

// An endpoint named across worlds: the world it lives in and its place there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Peer {
    pub(crate) world: NodeRef,
    pub(crate) endpoint: Local<Endpoint>,
}

// A moved reference as a read takes it out, with the world whose count it holds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Carried {
    pub(crate) object: ObjectRef,
    pub(crate) rights: Rights,
    pub(crate) origin: NodeRef,
}

pub(crate) type Received = [Option<Carried>; MESSAGE_HANDLES as usize];

pub(crate) const NONE_RECEIVED: Received = [None; MESSAGE_HANDLES as usize];

// A condemned chain with no endpoint on it.
pub(crate) const NO_ENDPOINT: u64 = u64::MAX;

// A ring read, named by its queue, slot and never-reused id.
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
    link: Link<Peer>,
    inbox: Inbox,
    // The byte ring, then each moved object's strong reference and its world, held until read,
    // then the caller of each descriptor that holds a call.
    bytes: ArenaFrame,
    armed: [Option<Reader>; ARMED],
    doomed: Option<Local<Endpoint>>,
}

const TRANSIT_AT: usize = BYTES as usize;

const TRANSIT: usize = ENCODED + 8;

const CALLS_AT: usize = TRANSIT_AT + HANDLES as usize * TRANSIT;

const FRAME_USED: usize = CALLS_AT + DESCRIPTORS as usize * ENCODED;

const COPY_CHUNK: usize = 256;

const _: () = assert!(
    FRAME_USED <= x86_paging::PAGE_SIZE as usize && FRAME_USED.is_multiple_of(COPY_CHUNK),
    "the byte ring and the moved references share one frame"
);

fn hand(frame: &mut ArenaFrame, at: usize, moved: Option<(ObjectRef, Rights)>, origin: NodeRef) {
    let mut encoded = [0; TRANSIT];
    if let Some((object, rights)) = moved {
        encoded[..ENCODED].copy_from_slice(&object.encode(rights));
        encoded[ENCODED..ENCODED + 4].copy_from_slice(&origin.node.to_le_bytes());
        encoded[ENCODED + 4..].copy_from_slice(&origin.generation.to_le_bytes());
    }
    assert!(
        frame.write_at(at, &encoded),
        "a transit slot left its frame"
    );
}

fn peek(frame: &ArenaFrame, at: usize) -> Option<Carried> {
    let mut encoded = [0; TRANSIT];
    assert!(
        frame.read_at(at, &mut encoded),
        "a transit slot left its frame"
    );
    let mut object = [0; ENCODED];
    object.copy_from_slice(&encoded[..ENCODED]);
    let word = |at: usize| {
        u32::from_le_bytes([
            encoded[at],
            encoded[at + 1],
            encoded[at + 2],
            encoded[at + 3],
        ])
    };
    ObjectRef::decode(object).map(|(object, rights)| Carried {
        object,
        rights,
        origin: NodeRef {
            node: word(ENCODED),
            generation: word(ENCODED + 4),
        },
    })
}

const NOWHERE: NodeRef = NodeRef {
    node: 0,
    generation: 0,
};

const _: () = assert!(
    HANDLES as usize * TRANSIT <= x86_paging::PAGE_SIZE as usize,
    "a reply's staged handles fit one frame"
);

// A reply's handles wait in its caller's staging frame in the form an inbox holds them.
pub(crate) fn stage(
    frame: &mut ArenaFrame,
    slot: usize,
    moved: Option<(ObjectRef, Rights)>,
    origin: NodeRef,
) {
    hand(frame, slot * TRANSIT, moved, origin);
}

pub(crate) fn unstage(frame: &mut ArenaFrame, slot: usize) -> Option<Carried> {
    let carried = peek(frame, slot * TRANSIT);
    hand(frame, slot * TRANSIT, None, NOWHERE);
    carried
}

impl State {
    fn hand(&mut self, slot: usize, moved: Option<(ObjectRef, Rights)>, origin: NodeRef) {
        hand(&mut self.bytes, TRANSIT_AT + slot * TRANSIT, moved, origin);
    }

    fn peek(&self, slot: usize) -> Option<Carried> {
        peek(&self.bytes, TRANSIT_AT + slot * TRANSIT)
    }

    fn claim(&mut self, slot: usize) -> Option<Carried> {
        let carried = self.peek(slot);
        self.hand(slot, None, NOWHERE);
        carried
    }

    fn mark(&mut self, index: u8, call: Option<Caller>) {
        let encoded = call.map_or([0; ENCODED], |caller| {
            ObjectRef::Reply(caller).encode(Rights::NONE)
        });
        assert!(
            self.bytes
                .write_at(CALLS_AT + usize::from(index) * ENCODED, &encoded),
            "a call mark left its frame"
        );
    }

    fn called(&self, index: u8) -> Option<Caller> {
        let mut encoded = [0; ENCODED];
        assert!(
            self.bytes
                .read_at(CALLS_AT + usize::from(index) * ENCODED, &mut encoded),
            "a call mark left its frame"
        );
        match ObjectRef::decode(encoded) {
            Some((ObjectRef::Reply(caller), _)) => Some(caller),
            _ => None,
        }
    }

    // The head entry and every moved reference it holds, with its caller when it is a call.
    fn pop(&mut self, received: &mut Received) -> Option<Option<Caller>> {
        let entry = self.inbox.pop()?;
        for (at, slot) in spanned(entry.descriptor.handle_spans()).enumerate() {
            received[at] = self.claim(slot);
        }
        Some(self.called(entry.index))
    }

    // Moved endpoints of the head message that live in another world and need a new home here.
    fn foreign(&self, entry: &channel_state::Entry, here: NodeRef) -> u16 {
        spanned(entry.descriptor.handle_spans())
            .filter(|&slot| {
                self.peek(slot).is_some_and(|carried| {
                    carried.origin != here && matches!(carried.object, ObjectRef::Endpoint(_))
                })
            })
            .count() as u16
    }

    fn closed(&self) -> bool {
        !matches!(self.link, Link::Open(_))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WriteRefusal {
    PeerClosed,
    Full,
    // An endpoint written into its own inbox, or an object that may not enter the peer's world.
    Denied,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Head {
    Empty,
    PeerClosed,
    Message {
        bytes: u16,
        handles: u16,
        foreign: u16,
        call: bool,
    },
}

// The world an operation runs for, or another world for one pinned step.
pub(crate) enum Reach<'a> {
    Own(&'a World),
    Pinned(Pin),
}

impl Deref for Reach<'_> {
    type Target = World;

    fn deref(&self) -> &World {
        match self {
            Self::Own(world) => world,
            Self::Pinned(pin) => pin,
        }
    }
}

pub(crate) fn reach(here: &World, node: NodeRef) -> Option<Reach<'_>> {
    if node == here.node() {
        Some(Reach::Own(here))
    } else {
        registry::pin(node, Admission::Live).map(Reach::Pinned)
    }
}

impl Reach<'_> {
    // Only the owner destroys at once; under a pin a last reference is left to the reaper.
    fn let_go(&self, endpoint: Local<Endpoint>) {
        match self {
            Self::Own(world) => release(world, endpoint),
            Self::Pinned(pin) => release_pinned(pin, endpoint),
        }
    }
}

impl Endpoint {
    pub(crate) fn frames_for(endpoints: u32) -> Option<u32> {
        EndpointPool::frames_for(endpoints)?.checked_add(endpoints)
    }

    // A re-home takes one more frame for the copy, which returns once the copy is in place.
    pub(crate) fn rehome_frames_for(endpoints: u32) -> Option<u32> {
        Self::frames_for(endpoints)?.checked_add(1)
    }

    fn insert(earmark: &mut Earmark, world: &World, link: Link<Peer>) -> Option<Local<Self>> {
        let bytes = earmark.take(FrameKind::ArenaData)?;
        Some(world.endpoints.insert(earmark, || Self {
            state: SpinLock::new(State {
                link,
                inbox: Inbox::EMPTY,
                bytes,
                armed: [None; ARMED],
                doomed: None,
            }),
        }))
    }
}

// Two endpoints in one world, each naming the other.
pub(crate) fn create(
    earmark: &mut Earmark,
    world: &World,
) -> Option<(Local<Endpoint>, Local<Endpoint>)> {
    let here = world.node();
    let first = Endpoint::insert(earmark, world, Link::PeerClosed)?;
    let Some(second) = Endpoint::insert(
        earmark,
        world,
        Link::Open(Peer {
            world: here,
            endpoint: first,
        }),
    ) else {
        release(world, first);
        return None;
    };
    held(world, first).state.lock_leaf().link = Link::Open(Peer {
        world: here,
        endpoint: second,
    });
    Some((first, second))
}

// The birth endpoint of a program: nothing is at the other end.
pub(crate) fn alone(earmark: &mut Earmark, world: &World) -> Option<Local<Endpoint>> {
    Endpoint::insert(earmark, world, Link::PeerClosed)
}

fn held(world: &World, endpoint: Local<Endpoint>) -> Hold<Endpoint> {
    world
        .endpoints
        .hold(endpoint)
        .unwrap_or_else(|| panic!("an endpoint went while a strong reference held it"))
}

enum Commit {
    Done(Option<Reader>),
    Follow(Peer),
}

// Commits one message into the peer's inbox, following one re-home, and hands back the oldest
// armed reader to wake. On a refusal the moved objects are still the caller's. A call's cell
// leaves Idle under the inbox lock, so no reader finds its entry before the call is queued.
pub(crate) fn write(
    world: &World,
    from: Local<Endpoint>,
    bytes: &[u8],
    moved: &mut Moved,
    count: usize,
    call: Option<(Caller, &CallCell)>,
) -> Result<Option<Reader>, WriteRefusal> {
    let me = Peer {
        world: world.node(),
        endpoint: from,
    };
    'resend: loop {
        let link = held(world, from).state.lock_leaf().link;
        let Link::Open(first) = link else {
            return Err(WriteRefusal::PeerClosed);
        };
        let (mut to, mut hop) = (first, Hop::First);
        loop {
            let there = reach(world, to.world).ok_or(WriteRefusal::PeerClosed)?;
            // The peer is reached only under a reference of its own, so it cannot go mid-write.
            if there.endpoints.acquire(to.endpoint).is_err() {
                drop(there);
                if hop == Hop::First && held(world, from).state.lock_leaf().link != link {
                    continue 'resend;
                }
                return Err(WriteRefusal::PeerClosed);
            }
            let committed = admit(&there, me.world, to, moved).and_then(|()| {
                let to = held(&there, to.endpoint);
                commit(&to, me, hop, (bytes, moved, count), call)
            });
            there.let_go(to.endpoint);
            match committed? {
                Commit::Done(reader) => return Ok(reader),
                Commit::Follow(next) => (to, hop) = (next, Hop::Forwarded),
            }
        }
    }
}

fn admit(there: &World, here: NodeRef, to: Peer, moved: &Moved) -> Result<(), WriteRefusal> {
    let own_inbox = to.world == here
        && moved
            .iter()
            .flatten()
            .any(|&(object, _)| object == ObjectRef::Endpoint(to.endpoint));
    if own_inbox || !admits(there, here, moved) {
        Err(WriteRefusal::Denied)
    } else {
        Ok(())
    }
}

// A memory object stays in its world, and a world handle enters only that world or an ancestor.
pub(crate) fn admits(there: &World, here: NodeRef, moved: &Moved) -> bool {
    let crossing = there.node() != here;
    !moved.iter().flatten().any(|&(object, _)| match object {
        ObjectRef::Memory(_) => crossing,
        ObjectRef::World(node) => crossing && !there.governs(node),
        ObjectRef::Endpoint(_)
        | ObjectRef::File(_)
        | ObjectRef::Directory(_)
        | ObjectRef::Reply(_)
        | ObjectRef::Dead(_) => false,
    })
}

fn commit(
    to: &Endpoint,
    from: Peer,
    hop: Hop,
    (bytes, moved, count): (&[u8], &mut Moved, usize),
    call: Option<(Caller, &CallCell)>,
) -> Result<Commit, WriteRefusal> {
    let mut state = to.state.lock_leaf();
    match route(from, state.link, hop) {
        Route::Deliver => {}
        Route::Follow(next) => return Ok(Commit::Follow(next)),
        Route::PeerClosed => return Err(WriteRefusal::PeerClosed),
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
        state.hand(slot, taken.next().flatten(), from.world);
    }
    state.mark(entry.index, call.map(|(caller, _)| caller));
    if let Some((caller, cell)) = call {
        cell.commit(caller);
    }
    Ok(Commit::Done(pop_armed(&mut state)))
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
            foreign: state.foreign(&entry, world.node()),
            call: state.called(entry.index).is_some(),
        },
        None if state.closed() => Head::PeerClosed,
        None => Head::Empty,
    }
}

#[cfg(feature = "selftest")]
pub(crate) fn queued(world: &World, endpoint: Local<Endpoint>) -> u8 {
    held(world, endpoint).state.lock_leaf().inbox.count()
}

// Pops the head message when it fits the room the reader holds, counting the moved endpoints it
// earmarked a new home for and the slot a call's reply right needs; `None` leaves it queued.
pub(crate) fn take(
    world: &World,
    endpoint: Local<Endpoint>,
    room: (u16, u16, u16, bool),
    into: &mut [u8],
    received: &mut Received,
) -> Option<(u16, u16, Option<Caller>)> {
    let hold = held(world, endpoint);
    let mut state = hold.state.lock_leaf();
    let entry = state.inbox.head()?;
    let (bytes, handles) = (entry.descriptor.bytes(), entry.descriptor.handles());
    if bytes > room.0
        || handles > room.1
        || state.foreign(&entry, world.node()) > room.2
        || (state.called(entry.index).is_some() && !room.3)
    {
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
    let caller = state.pop(received)?;
    Some((bytes, handles, caller))
}

fn copy_frame(from: &ArenaFrame, to: &mut ArenaFrame) {
    let mut chunk = [0; COPY_CHUNK];
    for at in (0..FRAME_USED).step_by(COPY_CHUNK) {
        assert!(
            from.read_at(at, &mut chunk) && to.write_at(at, &chunk),
            "an endpoint copy left its frame"
        );
    }
}

// A moved object as the reading world holds it: an endpoint of another world is copied in, and a
// counted object of another world, or one whose world died in transit, becomes a dead entry.
pub(crate) fn arrive(earmark: Option<&mut Earmark>, world: &World, carried: Carried) -> ObjectRef {
    if carried.origin == world.node() {
        return carried.object;
    }
    match carried.object {
        ObjectRef::Endpoint(endpoint) => earmark
            .and_then(|earmark| {
                let from = Peer {
                    world: carried.origin,
                    endpoint,
                };
                rehome(earmark, world, from)
            })
            .map_or(ObjectRef::Dead(ObjectType::Endpoint), ObjectRef::Endpoint),
        ObjectRef::Memory(_) => {
            if let Some(origin) = registry::pin(carried.origin, Admission::Live) {
                carried.object.release(&origin);
            }
            ObjectRef::Dead(ObjectType::Memory)
        }
        object => object,
    }
}

// A moved reference no table will hold: its count goes back to its world, and a reply right
// abandons its call.
pub(crate) fn drop_carried(world: &World, carried: Carried) {
    let mut doomed = None;
    dispose(world, carried, &mut doomed);
    if let Some(first) = doomed {
        destroy(world, first);
    }
}

fn dispose(world: &World, carried: Carried, doomed: &mut Option<Local<Endpoint>>) {
    match carried.object {
        ObjectRef::Reply(caller) => queue::abandon(world, caller),
        object if carried.origin == world.node() => discard(world, object, doomed),
        object => {
            if let Some(origin) = registry::pin(carried.origin, Admission::Live) {
                match object {
                    ObjectRef::Endpoint(moved) => release_pinned(&origin, moved),
                    object => object.release(&origin),
                }
            }
        }
    }
}

// Copies an endpoint of another world, its record and queued contents, into this world's arena
// from the earmark, then retires the old one; `None` when the other world is no longer live.
pub(crate) fn rehome(earmark: &mut Earmark, world: &World, from: Peer) -> Option<Local<Endpoint>> {
    let (Some(mut spare), Some(moved)) = (
        earmark.take(FrameKind::ArenaData),
        Endpoint::insert(earmark, world, Link::PeerClosed),
    ) else {
        panic!("a re-home outran its earmark");
    };
    let me = Peer {
        world: world.node(),
        endpoint: moved,
    };
    // The peer may still commit while the copy is made, so the old record is checked unchanged
    // before it forwards; each retry follows a commit into a bounded inbox or one link change.
    let link = loop {
        let Some(source) = registry::pin(from.world, Admission::Live) else {
            abandon(world, moved);
            return None;
        };
        let (inbox, link) = {
            let hold = held(&source, from.endpoint);
            let state = hold.state.lock_leaf();
            copy_frame(&state.bytes, &mut spare);
            (state.inbox.clone(), state.link)
        };
        drop(source);
        {
            let hold = held(world, moved);
            let mut state = hold.state.lock_leaf();
            core::mem::swap(&mut state.bytes, &mut spare);
            state.inbox = inbox.clone();
            state.link = link;
        }
        let Some(source) = registry::pin(from.world, Admission::Live) else {
            abandon(world, moved);
            return None;
        };
        let hold = held(&source, from.endpoint);
        let mut state = hold.state.lock_leaf();
        if state.inbox == inbox && state.link == link {
            state.link = Link::Forwarded(me);
            state.inbox = Inbox::EMPTY;
            break link;
        }
    };
    if let Link::Open(peer) = link {
        repoint(world, peer, from, me);
    }
    if let Some(source) = registry::pin(from.world, Admission::Live) {
        release_pinned(&source, from.endpoint);
    }
    drop(spare);
    Some(moved)
}

// An endpoint a spawn was given and never placed goes back through its own world.
pub(crate) fn forfeit(endpoint: Peer) {
    if let Some(world) = registry::pin(endpoint.world, Admission::Live) {
        release_pinned(&world, endpoint.endpoint);
    }
}

// A copy that never took over owns none of the references it holds.
fn abandon(world: &World, endpoint: Local<Endpoint>) {
    {
        let hold = held(world, endpoint);
        let mut state = hold.state.lock_leaf();
        state.inbox = Inbox::EMPTY;
        state.link = Link::PeerClosed;
    }
    release(world, endpoint);
}

// The peer names the new home instead of the old, following any re-home of its own.
fn repoint(world: &World, mut peer: Peer, old: Peer, new: Peer) {
    loop {
        let Some(there) = reach(world, peer.world) else {
            return;
        };
        if there.endpoints.acquire(peer.endpoint).is_err() {
            return;
        }
        let next = {
            let hold = held(&there, peer.endpoint);
            let mut state = hold.state.lock_leaf();
            match state.link {
                Link::Open(at) if at == old => {
                    state.link = Link::Open(new);
                    None
                }
                Link::Forwarded(to) => Some(to),
                Link::Open(_) | Link::PeerClosed => None,
            }
        };
        there.let_go(peer.endpoint);
        match next {
            Some(to) => peer = to,
            None => return,
        }
    }
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
    if state.inbox.count() > 0 || state.closed() {
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

// Under a pin of its world an endpoint that lost its last reference joins that world's condemned
// chain, which the reaper destroys as the owner, so no pinned step waits on another world.
fn release_pinned(world: &World, endpoint: Local<Endpoint>) {
    if let Ok(Released::NowZero) | Err(Refusal::Condemned) = world.endpoints.release(endpoint) {
        condemn(world, endpoint);
    }
}

fn condemn(world: &World, endpoint: Local<Endpoint>) {
    let mut head = world.condemned.load(Ordering::Acquire);
    loop {
        held(world, endpoint).state.lock_leaf().doomed =
            (head != NO_ENDPOINT).then(|| Local::from_bits(head));
        match world.condemned.compare_exchange(
            head,
            endpoint.to_bits(),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(now) => head = now,
        }
    }
    if head == NO_ENDPOINT {
        crate::reaper::submit_endpoints(world.node());
    }
}

fn condemned(world: &World) {
    let first = world.condemned.swap(NO_ENDPOINT, Ordering::AcqRel);
    if first != NO_ENDPOINT {
        destroy(world, Local::from_bits(first));
    }
}

// The reaper's step for a world whose endpoints lost their last reference under a pin.
pub(crate) fn collect(node: NodeRef) {
    if let Some(world) = registry::hold(node) {
        condemned(&world);
    }
}

// Iterative, since endpoints in transit can nest to any depth: each one that loses its last
// reference joins the doomed chain instead of being destroyed inside another's destruction.
fn destroy(world: &World, first: Local<Endpoint>) {
    let here = world.node();
    let mut doomed = Some(first);
    while let Some(endpoint) = doomed {
        let (link, armed) = {
            let held = held(world, endpoint);
            doomed = held.state.lock_leaf().doomed.take();
            loop {
                let mut received = NONE_RECEIVED;
                let Some(caller) = held.state.lock_leaf().pop(&mut received) else {
                    break;
                };
                if let Some(caller) = caller {
                    queue::close_call(world, caller);
                }
                for carried in received.into_iter().flatten() {
                    dispose(world, carried, &mut doomed);
                }
            }
            let mut state = held.state.lock_leaf();
            let armed = core::mem::replace(&mut state.armed, [None; ARMED]);
            (core::mem::replace(&mut state.link, Link::PeerClosed), armed)
        };
        wake(armed);
        if let Link::Open(peer) = link {
            close_toward(
                world,
                peer,
                Peer {
                    world: here,
                    endpoint,
                },
                &mut doomed,
            );
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

// Closes the peer toward the closed endpoint, following the peer's re-homes.
fn close_toward(world: &World, mut peer: Peer, closed: Peer, doomed: &mut Option<Local<Endpoint>>) {
    loop {
        let Some(there) = reach(world, peer.world) else {
            return;
        };
        if there.endpoints.acquire(peer.endpoint).is_err() {
            return;
        }
        let (armed, next) = {
            let hold = held(&there, peer.endpoint);
            let mut state = hold.state.lock_leaf();
            let next = match state.link {
                Link::Open(at) if at == closed => {
                    state.link = Link::PeerClosed;
                    None
                }
                Link::Forwarded(to) => Some(to),
                Link::Open(_) | Link::PeerClosed => None,
            };
            (core::mem::replace(&mut state.armed, [None; ARMED]), next)
        };
        wake(armed);
        match &there {
            Reach::Own(own) => discard(own, ObjectRef::Endpoint(peer.endpoint), doomed),
            Reach::Pinned(pin) => release_pinned(pin, peer.endpoint),
        }
        match next {
            Some(to) => peer = to,
            None => return,
        }
    }
}

fn wake(armed: [Option<Reader>; ARMED]) {
    for reader in armed.into_iter().flatten() {
        reader.queue.ready(reader);
    }
}

// A dead world's endpoints, those that reach each other only through their inboxes included,
// once no other world can pin it.
pub(crate) fn dissolve(world: &World) {
    condemned(world);
    while let Some(endpoint) = world.endpoints.any() {
        while let Ok(Released::Held) = world.endpoints.release(endpoint) {}
        destroy(world, endpoint);
    }
}
