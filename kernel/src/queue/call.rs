use core::sync::atomic::{AtomicU64, Ordering};

use channel_state::{CallEvent, CallState, Cell, CellRefusal};
use native_abi::Rights;
use native_abi::message::{BYTES_AT, DELIVERED, FLAGS_AT};
use native_abi::ring::{Completion, SUBMISSIONS, Submission, status};
use world_tree::NodeRef;
use wost::mm::pool::{self, ArenaFrame, Earmark, FrameKind};
use wost::sync::{Leaf, SpinLock};

use super::channel::{Room, describe, endpoint, header, install, message, room, send};
use super::{Pending, PendingKind, Process, Queue, QueueRef, answer, refuse, unresolved};
use crate::arena::Local;
use crate::channel::{self, Endpoint, Moved, NONE_MOVED, NONE_RECEIVED};
use crate::handle::Reservations;
use crate::object::ObjectRef;
use crate::world::World;

pub(crate) const RIGHTS: Rights = Rights::TRANSFER.union(Rights::WRITE);

pub(super) const NEVER: u64 = u64::MAX;

const SLOT_BITS: u32 = 3;

const _: () = assert!(
    SUBMISSIONS <= 1 << SLOT_BITS,
    "a call names its pending slot in three bits"
);

// A call names its caller's queue, the pending slot it waits in and that slot's cell generation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Caller {
    queue: QueueRef,
    call: u64,
}

impl Caller {
    fn new(queue: QueueRef, slot: usize, generation: u64) -> Self {
        Self {
            queue,
            call: generation << SLOT_BITS | slot as u64,
        }
    }

    fn slot(self) -> usize {
        (self.call & ((1 << SLOT_BITS) - 1)) as usize
    }

    fn generation(self) -> u64 {
        self.call >> SLOT_BITS
    }

    pub(crate) fn to_words(self) -> [u64; 3] {
        let world = self.queue.world;
        [
            u64::from(world.node) << u32::BITS | u64::from(world.generation),
            self.queue.queue.to_bits(),
            self.call,
        ]
    }

    pub(crate) fn from_words([world, queue, call]: [u64; 3]) -> Self {
        Self {
            queue: QueueRef {
                world: NodeRef {
                    node: (world >> u32::BITS) as u32,
                    generation: world as u32,
                },
                queue: Local::from_bits(queue),
            },
            call,
        }
    }
}

// The word of a `channel_state::Cell`: one call at a time per pending slot.
#[derive(Debug)]
pub(crate) struct CallCell(AtomicU64);

impl CallCell {
    fn load(&self) -> Cell {
        Cell::from_bits(self.0.load(Ordering::Acquire))
            .unwrap_or_else(|| panic!("a call cell held no state"))
    }

    // The state the event moved the cell from.
    fn step(&self, event: CallEvent, generation: u64) -> Result<CallState, CellRefusal> {
        let mut seen = self.0.load(Ordering::Acquire);
        loop {
            let Some(cell) = Cell::from_bits(seen) else {
                panic!("a call cell held no state");
            };
            let next = cell.next(event, generation)?;
            match self.0.compare_exchange_weak(
                seen,
                next.bits(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(cell.state()),
                Err(now) => seen = now,
            }
        }
    }

    pub(crate) fn commit(&self, caller: Caller) {
        assert!(
            self.step(CallEvent::Commit, caller.generation()).is_ok(),
            "a call committed from a cell that was not idle"
        );
    }
}

#[derive(Debug)]
pub(super) struct CallSlot {
    cell: CallCell,
    stage: SpinLock<Leaf, Option<Stage>>,
}

impl CallSlot {
    pub(super) fn new() -> Self {
        Self {
            cell: CallCell(AtomicU64::new(Cell::IDLE.bits())),
            stage: SpinLock::new(None),
        }
    }
}

// What a caller prepaid at admission, and what its reply staged there.
#[derive(Debug)]
struct Stage {
    at: u64,
    room: Room,
    replied: (u16, u16),
    handles: Option<ArenaFrame>,
    earmark: Option<Earmark>,
    slots: Reservations,
}

pub(super) fn call(
    queue: &Queue,
    mut process: Process<'_, '_>,
    submission: Submission,
) -> Option<Completion> {
    let token = submission.token;
    if submission.argument != NEVER && !crate::time::is_calibrated() {
        return Some(refuse(token, status::UNAVAILABLE));
    }
    let parsed = header(queue, &submission).and_then(|header| {
        Ok((
            room(&header, submission.at)?,
            message(queue, &header, submission.at)?,
        ))
    });
    let (room, message) = match parsed {
        Ok(parsed) => parsed,
        Err(code) => return Some(refuse(token, code)),
    };
    let to = match endpoint(&mut process, submission.handle, Rights::WRITE) {
        Ok(endpoint) => endpoint,
        Err(code) => return Some(refuse(token, code)),
    };
    let Ok(slots) = process.table_ref.reserve_slots(room.handles) else {
        return Some(refuse(token, status::NO_HANDLE_SLOT));
    };
    let Some((handles, earmark)) = prepay(&mut process, room.handles) else {
        return Some(refuse(token, status::NO_MEMORY));
    };
    let deadline_ns = if submission.argument == NEVER {
        NEVER
    } else {
        queue.clock.to_host_ns(submission.argument)
    };
    let kind = PendingKind::ChannelCall {
        deadline_ns,
        ready: false,
    };
    let Some((index, entry)) = queue.park(token, kind) else {
        return Some(refuse(token, status::NO_ROOM));
    };
    let call = &queue.calls[index];
    let cell = call.cell.load();
    assert!(
        cell.state() == CallState::Idle,
        "a free pending slot held a call"
    );
    let caller = Caller::new(process.here, index, cell.generation());
    *call.stage.lock_leaf() = Some(Stage {
        at: submission.at,
        room,
        replied: (0, 0),
        handles,
        earmark,
        slots,
    });
    match send(&mut process, to, &message, Some((caller, &call.cell))) {
        Ok(()) => {
            queue.unclaim(index, entry);
            None
        }
        Err(code) => {
            let stage = call.stage.lock_leaf().take();
            drop(stage);
            queue.unpark(index, entry);
            Some(refuse(token, code))
        }
    }
}

// The staging frame and every re-home the reply's handles could need, from the caller's arena.
fn prepay(
    process: &mut Process<'_, '_>,
    handles: u16,
) -> Option<(Option<ArenaFrame>, Option<Earmark>)> {
    if handles == 0 {
        return Some((None, None));
    }
    let frames = Endpoint::rehome_frames_for(u32::from(handles))?.checked_add(1)?;
    let mut earmark = pool::earmark(process.context, process.world.arena(), frames).ok()?;
    let staging = earmark.take(FrameKind::ArenaData)?;
    Some((Some(staging), Some(earmark)))
}

// The right is spent by a reply and by a refused claim; a refusal of the reply itself keeps it.
pub(super) fn reply(queue: &Queue, process: Process<'_, '_>, submission: Submission) -> Completion {
    let token = submission.token;
    let message = match header(queue, &submission)
        .and_then(|header| message(queue, &header, submission.at))
    {
        Ok(message) => message,
        Err(code) => return refuse(token, code),
    };
    let caller = match process.table.resolve(process.context, submission.handle) {
        Ok((ObjectRef::Reply(caller), rights)) if rights.contains(Rights::WRITE) => caller,
        Ok(_) => return refuse(token, status::DENIED),
        Err(error) => return refuse(token, unresolved(error)),
    };
    let named = message.named();
    let mut moved = NONE_MOVED;
    if process
        .table
        .take(process.context, named, &mut moved)
        .is_err()
    {
        return refuse(token, status::BAD_HANDLE);
    }
    let claimed = claim(
        process.world,
        caller,
        message.bytes(),
        &mut moved,
        named.len(),
    );
    if claimed.is_ok() {
        process.table.erase(process.context, named);
    } else {
        process.table.restore(process.context, named);
    }
    if matches!(claimed, Ok(()) | Err(Unanswered::Gone)) {
        let _ = process.table.spend(process.context, submission.handle);
    }
    match claimed {
        Ok(()) => answer(token, 0),
        Err(Unanswered::Gone) => refuse(token, status::PEER_CLOSED),
        Err(Unanswered::Denied) => refuse(token, status::DENIED),
        Err(Unanswered::TooSmall(room)) => Completion {
            token,
            status: status::TOO_SMALL,
            value: u64::from(room.bytes),
            extra: u64::from(room.handles),
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Unanswered {
    // The call ended or its caller is gone.
    Gone,
    // A moved handle may not enter the caller's world.
    Denied,
    // The reply exceeds the room the caller declared.
    TooSmall(Room),
}

// Claim, copy and complete in one step under at most one pin of the caller's world; the step
// moves only the cell and fills what the caller prepaid.
fn claim(
    world: &World,
    caller: Caller,
    bytes: &[u8],
    moved: &mut Moved,
    count: usize,
) -> Result<(), Unanswered> {
    let there = channel::reach(world, caller.queue.world).ok_or(Unanswered::Gone)?;
    if !channel::admits(&there, world.node(), moved) {
        return Err(Unanswered::Denied);
    }
    let queue = there
        .queues
        .hold(caller.queue.queue)
        .ok_or(Unanswered::Gone)?;
    let call = queue.calls.get(caller.slot()).ok_or(Unanswered::Gone)?;
    if call.cell.load().generation() != caller.generation() {
        return Err(Unanswered::Gone);
    }
    let (at, room) = call
        .stage
        .lock_leaf()
        .as_ref()
        .map(|stage| (stage.at, stage.room))
        .ok_or(Unanswered::Gone)?;
    if bytes.len() > usize::from(room.bytes) || count > usize::from(room.handles) {
        return Err(Unanswered::TooSmall(room));
    }
    call.cell
        .step(CallEvent::Claim, caller.generation())
        .map_err(|_| Unanswered::Gone)?;
    let copied = bytes.is_empty()
        || usize::try_from(at).is_ok_and(|at| queue.write_data(at + BYTES_AT, bytes));
    assert!(copied, "a reply left the room its caller declared");
    {
        let mut stage = call.stage.lock_leaf();
        let Some(stage) = stage.as_mut() else {
            panic!("a claimed call lost its staging");
        };
        if let Some(frame) = &mut stage.handles {
            for (slot, moved) in moved.iter_mut().take(count).enumerate() {
                channel::stage(frame, slot, moved.take(), world.node());
            }
        }
        stage.replied = (bytes.len() as u16, count as u16);
    }
    assert!(
        call.cell
            .step(CallEvent::Complete, caller.generation())
            .is_ok(),
        "a claimed reply did not complete"
    );
    queue.ready_call(caller.slot(), caller.generation());
    Ok(())
}

// One transition of the caller's cell under at most one pin of its world; an outcome wakes it.
fn settle(world: &World, caller: Caller, event: CallEvent) -> bool {
    let Some(there) = channel::reach(world, caller.queue.world) else {
        return false;
    };
    let Some(queue) = there.queues.hold(caller.queue.queue) else {
        return false;
    };
    let Some(call) = queue.calls.get(caller.slot()) else {
        return false;
    };
    if call.cell.step(event, caller.generation()).is_err() {
        return false;
    }
    if event != CallEvent::Deliver {
        queue.ready_call(caller.slot(), caller.generation());
    }
    true
}

// `false` when the caller gave up while the call was queued.
pub(crate) fn deliver(world: &World, caller: Caller) -> bool {
    settle(world, caller, CallEvent::Deliver)
}

// The endpoint that held the queued call is destroyed.
pub(crate) fn close_call(world: &World, caller: Caller) {
    settle(world, caller, CallEvent::Close);
}

// The reply right is dropped.
pub(crate) fn abandon(world: &World, caller: Caller) {
    settle(world, caller, CallEvent::Abandon);
}

fn consume(call: &CallSlot, generation: u64) -> Stage {
    let Some(stage) = call.stage.lock_leaf().take() else {
        panic!("a call ended without its staging");
    };
    assert!(
        call.cell.step(CallEvent::Consume, generation).is_ok(),
        "a call cell ran out of generations"
    );
    stage
}

// The caller's own cancel or deadline; nothing was staged, so the outcome is posted at once.
pub(super) fn end(
    queue: &Queue,
    index: usize,
    entry: Pending,
    event: CallEvent,
) -> Option<Completion> {
    let call = &queue.calls[index];
    let generation = call.cell.load().generation();
    let from = call.cell.step(event, generation).ok()?;
    let stage = consume(call, generation);
    let flags = if from == CallState::Delivered {
        DELIVERED
    } else {
        0
    };
    let flagged = usize::try_from(stage.at)
        .is_ok_and(|at| queue.write_data(at + FLAGS_AT, &flags.to_le_bytes()));
    assert!(flagged, "a call's descriptor left its buffer");
    drop(stage);
    let code = if event == CallEvent::Timeout {
        status::TIMED_OUT
    } else {
        status::CANCELLED
    };
    Some(refuse(entry.token, code))
}

// An outcome the peer reached, taken in the caller's context: the reply's handles are installed
// from what the caller prepaid.
pub(super) fn serve(
    queue: &Queue,
    mut process: Process<'_, '_>,
    index: usize,
    entry: Pending,
) -> Completion {
    let call = &queue.calls[index];
    let cell = call.cell.load();
    let code = match cell.state() {
        CallState::Replied => status::OK,
        CallState::Abandoned => status::ABANDONED,
        CallState::PeerClosed => status::PEER_CLOSED,
        _ => panic!("a call was readied without an outcome from its peer"),
    };
    let mut stage = consume(call, cell.generation());
    if code != status::OK {
        return refuse(entry.token, code);
    }
    let (bytes, handles) = stage.replied;
    let mut received = NONE_RECEIVED;
    if let Some(frame) = &mut stage.handles {
        for (slot, carried) in received.iter_mut().take(usize::from(handles)).enumerate() {
            *carried = channel::unstage(frame, slot);
        }
    }
    let installed = install(
        &mut process,
        &mut stage.slots,
        &mut received,
        stage.earmark.as_mut(),
    );
    let installed = &installed[..usize::from(handles)];
    if !describe(queue, stage.at, usize::from(bytes), installed, 0) {
        return refuse(entry.token, status::BAD_BUFFER);
    }
    Completion {
        token: entry.token,
        status: status::OK,
        value: u64::from(bytes),
        extra: u64::from(handles),
    }
}

// A dying caller's call its peer already decided: the staged handles go back to their worlds.
// `None` while a reply is still being copied in.
pub(super) fn forget(
    queue: &Queue,
    world: &World,
    index: usize,
    entry: Pending,
) -> Option<Completion> {
    let call = &queue.calls[index];
    let cell = call.cell.load();
    if !cell.state().is_outcome() {
        return None;
    }
    let mut stage = consume(call, cell.generation());
    if cell.state() == CallState::Replied
        && let Some(frame) = &mut stage.handles
    {
        for slot in 0..usize::from(stage.replied.1) {
            if let Some(carried) = channel::unstage(frame, slot) {
                channel::drop_carried(world, carried);
            }
        }
    }
    drop(stage);
    Some(refuse(entry.token, status::CANCELLED))
}

impl Queue {
    // Checked against the cell under the pending lock, so a stale wake never reaches a later call.
    fn ready_call(&self, slot: usize, generation: u64) {
        {
            let mut pending = self.pending.lock_leaf();
            let current = self
                .calls
                .get(slot)
                .map(|call| call.cell.load().generation());
            let Some(Some(Pending {
                kind: PendingKind::ChannelCall { ready, .. },
                ..
            })) = pending.slots.get_mut(slot)
            else {
                return;
            };
            if current != Some(generation) {
                return;
            }
            *ready = true;
        }
        self.readied.fetch_add(1, Ordering::Release);
        self.waiters.wake_all();
    }

    pub(super) fn unclaim(&self, index: usize, entry: Pending) {
        let mut pending = self.pending.lock_leaf();
        let Some(Some(current)) = pending.slots.get_mut(index) else {
            panic!("a claimed call lost its pending entry");
        };
        assert!(current.id == entry.id, "a claimed call's slot was reused");
        current.settling = false;
    }

    // A deadline that passed after the peer decided the call no longer applies.
    pub(super) fn outlive(&self, index: usize, entry: Pending) {
        let mut pending = self.pending.lock_leaf();
        let Some(Some(current)) = pending.slots.get_mut(index) else {
            panic!("a claimed call lost its pending entry");
        };
        assert!(current.id == entry.id, "a claimed call's slot was reused");
        if let PendingKind::ChannelCall { deadline_ns, .. } = &mut current.kind {
            *deadline_ns = NEVER;
        }
        current.settling = false;
    }
}
