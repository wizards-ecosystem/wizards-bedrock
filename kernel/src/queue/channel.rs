use native_abi::Rights;
use native_abi::limits::{MESSAGE_BYTES, MESSAGE_HANDLES};
use native_abi::message::{
    BYTE_CAPACITY, BYTES, BYTES_AT, ENTRIES_AT, ENTRY_BYTES, FLAGS_AT, HANDLE_CAPACITY, HANDLES,
    REPLY, VERSION, VERSION_AT,
};
use native_abi::ring::{Completion, DATA_BYTES, Submission, status};
use wost::mm::pool;

use super::call::{self, CallCell, Caller};
use super::{Pending, PendingKind, Process, Queue, QueueRef, answer, refuse, unresolved};
use crate::arena::Local;
use crate::channel::{
    self, ArmRefusal, Endpoint, Head, NONE_MOVED, NONE_RECEIVED, RIGHTS, Reader, Received,
    WriteRefusal,
};
use crate::handle::Reservations;
use crate::object::ObjectRef;

const HEADER: usize = ENTRIES_AT;

pub(super) fn create(process: Process<'_, '_>, submission: Submission) -> Completion {
    let token = submission.token;
    let (Ok(first_slot), Ok(second_slot)) = (
        process.table_ref.reserve_slot(),
        process.table_ref.reserve_slot(),
    ) else {
        return refuse(token, status::NO_HANDLE_SLOT);
    };
    let Some(frames) = Endpoint::frames_for(2) else {
        return refuse(token, status::NO_MEMORY);
    };
    let Ok(mut earmark) = pool::earmark(process.context, process.world.arena(), frames) else {
        return refuse(token, status::NO_MEMORY);
    };
    let Some((first, second)) = channel::create(&mut earmark, process.world) else {
        return refuse(token, status::NO_MEMORY);
    };
    drop(earmark);
    let installed = [(first, first_slot), (second, second_slot)].map(|(endpoint, slot)| {
        let object = ObjectRef::Endpoint(endpoint);
        slot.install(process.context, object, RIGHTS, RIGHTS)
            .map_err(|_| object.release(process.world))
    });
    match installed {
        [Ok(first), Ok(second)] => Completion {
            token,
            status: status::OK,
            value: first,
            extra: second,
        },
        [first, second] => {
            for handle in [first, second].into_iter().flatten() {
                let _ = process.table.close(process.context, process.world, handle);
            }
            refuse(token, status::BAD_HANDLE)
        }
    }
}

fn read_bytes(queue: &Queue, at: u64, into: &mut [u8]) -> bool {
    let Ok(length) = u32::try_from(into.len()) else {
        return false;
    };
    let bounds = Submission::about(native_abi::ring::Opcode::Echo, 0, 0, 0).bytes(0, at, length);
    into.is_empty() || queue.read_data(&bounds, into) == Some(into.len())
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

// Only buffer 0 exists until buffers can be registered.
pub(super) fn header(queue: &Queue, submission: &Submission) -> Result<[u8; HEADER], u32> {
    let mut header = [0; HEADER];
    if submission.buffer != 0
        || !submission.at.is_multiple_of(8)
        || !read_bytes(queue, submission.at, &mut header)
    {
        return Err(status::BAD_BUFFER);
    }
    if word(&header, VERSION_AT) != VERSION || word(&header, FLAGS_AT) != 0 {
        return Err(status::DENIED);
    }
    Ok(header)
}

pub(super) fn endpoint(
    process: &mut Process<'_, '_>,
    handle: u64,
    right: Rights,
) -> Result<Local<Endpoint>, u32> {
    match process.table.resolve(process.context, handle) {
        Ok((ObjectRef::Endpoint(endpoint), rights)) if rights.contains(right) => Ok(endpoint),
        Ok(_) => Err(status::DENIED),
        Err(error) => Err(unresolved(error)),
    }
}

// A message a program wrote: its bytes and the handles it names to move.
pub(super) struct Message {
    bytes: [u8; MESSAGE_BYTES as usize],
    length: usize,
    named: [(u64, Rights); MESSAGE_HANDLES as usize],
    count: usize,
}

impl Message {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    pub(super) fn named(&self) -> &[(u64, Rights)] {
        &self.named[..self.count]
    }
}

pub(super) fn message(queue: &Queue, header: &[u8; HEADER], at: u64) -> Result<Message, u32> {
    let (length, count) = (word(header, BYTES), word(header, HANDLES));
    if length > MESSAGE_BYTES || count > MESSAGE_HANDLES {
        return Err(status::LIMIT);
    }
    let mut message = Message {
        bytes: [0; MESSAGE_BYTES as usize],
        length: length as usize,
        named: [(0, Rights::NONE); MESSAGE_HANDLES as usize],
        count: count as usize,
    };
    let mut entries = [0; MESSAGE_HANDLES as usize * ENTRY_BYTES];
    let entries = &mut entries[..message.count * ENTRY_BYTES];
    if !read_bytes(queue, at + HEADER as u64, entries)
        || !read_bytes(
            queue,
            at + BYTES_AT as u64,
            &mut message.bytes[..message.length],
        )
    {
        return Err(status::BAD_BUFFER);
    }
    for (slot, entry) in message
        .named
        .iter_mut()
        .zip(entries.chunks_exact(ENTRY_BYTES))
    {
        let mut handle = [0; 8];
        handle.copy_from_slice(&entry[..8]);
        let Some(rights) = Rights::from_bits(word(entry, 8)).filter(|_| word(entry, 12) == 0)
        else {
            return Err(status::DENIED);
        };
        *slot = (u64::from_le_bytes(handle), rights);
    }
    Ok(message)
}

pub(super) fn write(
    queue: &Queue,
    mut process: Process<'_, '_>,
    submission: Submission,
) -> Completion {
    let token = submission.token;
    let message = match header(queue, &submission)
        .and_then(|header| message(queue, &header, submission.at))
    {
        Ok(message) => message,
        Err(code) => return refuse(token, code),
    };
    let from = match endpoint(&mut process, submission.handle, Rights::WRITE) {
        Ok(endpoint) => endpoint,
        Err(code) => return refuse(token, code),
    };
    match send(&mut process, from, &message, None) {
        Ok(()) => answer(token, 0),
        Err(code) => refuse(token, code),
    }
}

// Commits the message with the handles it names and wakes its reader; a refusal leaves every
// handle in the table with its value and rights.
pub(super) fn send(
    process: &mut Process<'_, '_>,
    from: Local<Endpoint>,
    message: &Message,
    call: Option<(Caller, &CallCell)>,
) -> Result<(), u32> {
    let named = message.named();
    let mut moved = NONE_MOVED;
    if process
        .table
        .take(process.context, named, &mut moved)
        .is_err()
    {
        return Err(status::BAD_HANDLE);
    }
    match channel::write(
        process.world,
        from,
        message.bytes(),
        &mut moved,
        named.len(),
        call,
    ) {
        Ok(reader) => {
            process.table.erase(process.context, named);
            if let Some(reader) = reader {
                reader.queue.ready(reader);
            }
            Ok(())
        }
        Err(refusal) => {
            process.table.restore(process.context, named);
            Err(match refusal {
                WriteRefusal::PeerClosed => status::PEER_CLOSED,
                WriteRefusal::Full => status::FULL,
                WriteRefusal::Denied => status::DENIED,
            })
        }
    }
}

// The room a reader or caller declared, capped at what one message can hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Room {
    pub(super) bytes: u16,
    pub(super) handles: u16,
}

pub(super) fn room(header: &[u8; HEADER], at: u64) -> Result<Room, u32> {
    let bytes = word(header, BYTE_CAPACITY).min(MESSAGE_BYTES);
    let handles = word(header, HANDLE_CAPACITY).min(MESSAGE_HANDLES);
    let end = usize::try_from(at)
        .ok()
        .and_then(|at| at.checked_add(BYTES_AT + bytes as usize));
    if end.is_none_or(|end| end > DATA_BYTES) {
        return Err(status::BAD_BUFFER);
    }
    Ok(Room {
        bytes: bytes as u16,
        handles: handles as u16,
    })
}

enum Outcome {
    Done(Completion),
    Armed,
}

pub(super) fn read(
    queue: &Queue,
    mut process: Process<'_, '_>,
    submission: Submission,
) -> Option<Completion> {
    let token = submission.token;
    if let Some(completion) = attempt(queue, &mut process, token, &submission) {
        return Some(completion);
    }
    let endpoint = match endpoint(&mut process, submission.handle, Rights::READ) {
        Ok(endpoint) => endpoint,
        Err(code) => return Some(refuse(token, code)),
    };
    let kind = PendingKind::ChannelRead {
        handle: submission.handle,
        at: submission.at,
        endpoint,
        ready: false,
    };
    let Some((index, entry)) = queue.park(token, kind) else {
        return Some(refuse(token, status::NO_ROOM));
    };
    match settle(queue, process, index, entry) {
        Outcome::Armed => None,
        Outcome::Done(completion) => {
            queue.unpark(index, entry);
            Some(completion)
        }
    }
}

// A read that became ready, served in the reader's own context.
pub(super) fn serve_ready(
    queue: &Queue,
    process: Process<'_, '_>,
    index: usize,
    entry: Pending,
) -> Option<Completion> {
    match settle(queue, process, index, entry) {
        Outcome::Done(completion) => Some(completion),
        Outcome::Armed => None,
    }
}

// Reads now, or arms the parked entry; a message that lands between the two is read at once.
fn settle(queue: &Queue, mut process: Process<'_, '_>, index: usize, entry: Pending) -> Outcome {
    let PendingKind::ChannelRead {
        handle,
        at,
        endpoint,
        ..
    } = entry.kind
    else {
        panic!("a channel read settled another operation");
    };
    let submission = Submission {
        at,
        ..Submission::about(
            native_abi::ring::Opcode::ChannelRead,
            entry.token,
            handle,
            0,
        )
    };
    loop {
        if let Some(completion) = attempt(queue, &mut process, entry.token, &submission) {
            return Outcome::Done(completion);
        }
        let reader = Reader {
            queue: process.here,
            slot: index,
            id: entry.id,
        };
        // Unclaimed before arming, so a write that readies it is never overwritten.
        queue.rearm(index, entry);
        let armed = channel::arm(process.world, endpoint, reader);
        if armed.is_ok() || !queue.reclaim(index, entry) {
            return Outcome::Armed;
        }
        if armed == Err(ArmRefusal::Full) {
            return Outcome::Done(refuse(entry.token, status::NO_ROOM));
        }
    }
}

// `None` when the inbox is empty and the peer still open, so the read must wait. A call whose
// caller gave up while it was queued is discarded with its handles, and the next entry is read.
fn attempt(
    queue: &Queue,
    process: &mut Process<'_, '_>,
    token: u32,
    submission: &Submission,
) -> Option<Completion> {
    let at = submission.at;
    let room = match header(queue, submission).and_then(|header| room(&header, at)) {
        Ok(room) => room,
        Err(code) => return Some(refuse(token, code)),
    };
    let endpoint = match endpoint(process, submission.handle, Rights::READ) {
        Ok(endpoint) => endpoint,
        Err(code) => return Some(refuse(token, code)),
    };
    loop {
        let (bytes, handles, foreign, call) = match channel::head(process.world, endpoint) {
            Head::Empty => return None,
            Head::PeerClosed => return Some(refuse(token, status::PEER_CLOSED)),
            Head::Message {
                bytes,
                handles,
                foreign,
                call,
            } => (bytes, handles, foreign, call),
        };
        if bytes > room.bytes || handles > room.handles {
            return Some(Completion {
                token,
                status: status::TOO_SMALL,
                value: u64::from(bytes),
                extra: u64::from(handles),
            });
        }
        // The reader's world pays for every moved endpoint that comes from another world.
        let mut earmark = None;
        if foreign > 0 {
            let frames = Endpoint::rehome_frames_for(u32::from(foreign));
            match frames.map(|frames| pool::earmark(process.context, process.world.arena(), frames))
            {
                Some(Ok(taken)) => earmark = Some(taken),
                Some(Err(_)) | None => return Some(refuse(token, status::NO_MEMORY)),
            }
        }
        let Ok(mut slots) = process.table_ref.reserve_slots(handles + u16::from(call)) else {
            return Some(refuse(token, status::NO_HANDLE_SLOT));
        };
        let mut payload = [0; MESSAGE_BYTES as usize];
        let mut received = NONE_RECEIVED;
        let Some((bytes, handles, caller)) = channel::take(
            process.world,
            endpoint,
            (room.bytes, handles, foreign, call),
            &mut payload,
            &mut received,
        ) else {
            continue;
        };
        if let Some(caller) = caller
            && !call::deliver(process.world, caller)
        {
            for carried in received.into_iter().flatten() {
                channel::drop_carried(process.world, carried);
            }
            continue;
        }
        let installed = install(process, &mut slots, &mut received, earmark.as_mut());
        let reply = caller.map_or(0, |caller| mint(process, &mut slots, caller));
        if !deliver(
            queue,
            at,
            &payload[..usize::from(bytes)],
            &installed[..usize::from(handles)],
            reply,
        ) {
            return Some(refuse(token, status::BAD_BUFFER));
        }
        return Some(Completion {
            token,
            status: status::OK,
            value: u64::from(bytes),
            extra: u64::from(handles),
        });
    }
}

// The right to answer a received call, in the slot its read reserved; a right no slot takes is
// dropped, which abandons the call.
fn mint(process: &mut Process<'_, '_>, slots: &mut Reservations, caller: Caller) -> u64 {
    let right = ObjectRef::Reply(caller);
    match slots
        .next()
        .map(|slot| slot.install(process.context, right, call::RIGHTS, call::RIGHTS))
    {
        Some(Ok(handle)) => handle,
        Some(Err(_)) | None => {
            right.release(process.world);
            0
        }
    }
}

pub(super) fn install(
    process: &mut Process<'_, '_>,
    slots: &mut Reservations,
    received: &mut Received,
    mut earmark: Option<&mut pool::Earmark>,
) -> [(u64, Rights); MESSAGE_HANDLES as usize] {
    let mut installed = [(0, Rights::NONE); MESSAGE_HANDLES as usize];
    for (carried, out) in received.iter_mut().zip(&mut installed) {
        let Some(carried) = carried.take() else {
            continue;
        };
        let object = channel::arrive(earmark.as_deref_mut(), process.world, carried);
        let rights = carried.rights;
        match slots
            .next()
            .map(|slot| slot.install(process.context, object, rights, rights))
        {
            Some(Ok(handle)) => *out = (handle, rights),
            Some(Err(_)) | None => object.release(process.world),
        }
    }
    installed
}

fn deliver(queue: &Queue, at: u64, payload: &[u8], handles: &[(u64, Rights)], reply: u64) -> bool {
    usize::try_from(at).is_ok_and(|start| {
        describe(queue, at, payload.len(), handles, reply)
            && queue.write_data(start + BYTES_AT, payload)
    })
}

// The counts, the reply right and each installed handle, around bytes already in place.
pub(super) fn describe(
    queue: &Queue,
    at: u64,
    bytes: usize,
    handles: &[(u64, Rights)],
    reply: u64,
) -> bool {
    let Ok(at) = usize::try_from(at) else {
        return false;
    };
    let mut header = [0u8; HEADER];
    header[BYTES..BYTES + 4].copy_from_slice(&(bytes as u32).to_le_bytes());
    header[HANDLES..HANDLES + 4].copy_from_slice(&(handles.len() as u32).to_le_bytes());
    let mut written = queue.write_data(at + BYTES, &header[BYTES..HANDLES + 4])
        && queue.write_data(at + REPLY, &reply.to_le_bytes());
    for (index, (handle, rights)) in handles.iter().enumerate() {
        let mut entry = [0u8; ENTRY_BYTES];
        entry[..8].copy_from_slice(&handle.to_le_bytes());
        entry[8..12].copy_from_slice(&rights.bits().to_le_bytes());
        written &= queue.write_data(at + ENTRIES_AT + index * ENTRY_BYTES, &entry);
    }
    written
}

impl QueueRef {
    // A write made this reader's endpoint readable; the reader reads at its next ring wait.
    pub(crate) fn ready(self, reader: Reader) {
        let Some(queue) = self.hold() else {
            return;
        };
        {
            let mut pending = queue.pending.lock_leaf();
            let Some(Some(Pending {
                id,
                kind: PendingKind::ChannelRead { ready, .. },
                ..
            })) = pending.slots.get_mut(reader.slot)
            else {
                return;
            };
            if *id != reader.id {
                return;
            }
            *ready = true;
        }
        queue
            .readied
            .fetch_add(1, core::sync::atomic::Ordering::Release);
        queue.waiters.wake_all();
    }
}

impl Queue {
    // Claimed from the start, so no other path settles it while its read is attempted.
    pub(super) fn park(&self, token: u32, kind: PendingKind) -> Option<(usize, Pending)> {
        let mut pending = self.pending.lock_leaf();
        let (index, entry) = pending.insert(token, kind)?;
        let claimed = Pending {
            settling: true,
            ..entry
        };
        pending.slots[index] = Some(claimed);
        Some((index, claimed))
    }

    pub(super) fn unpark(&self, index: usize, entry: Pending) {
        let mut pending = self.pending.lock_leaf();
        assert!(
            pending.slots[index].is_some_and(|current| current.id == entry.id),
            "a parked read lost its pending entry"
        );
        pending.slots[index] = None;
    }

    fn rearm(&self, index: usize, entry: Pending) {
        let mut pending = self.pending.lock_leaf();
        let Some(Some(current)) = pending.slots.get_mut(index) else {
            panic!("an armed read lost its pending entry");
        };
        assert!(current.id == entry.id, "an armed read's slot was reused");
        current.settling = false;
        if let PendingKind::ChannelRead { ready, .. } = &mut current.kind {
            *ready = false;
        }
    }

    // `false` when another path claimed the entry since it was armed.
    fn reclaim(&self, index: usize, entry: Pending) -> bool {
        let mut pending = self.pending.lock_leaf();
        match pending.slots.get_mut(index) {
            Some(Some(current)) if current.id == entry.id && !current.settling => {
                current.settling = true;
                true
            }
            _ => false,
        }
    }

    // A settled read that was still armed leaves its endpoint's readers.
    pub(super) fn disarm(here: QueueRef, index: usize, entry: Pending) {
        let PendingKind::ChannelRead { endpoint, .. } = entry.kind else {
            return;
        };
        if let Some(world) = crate::world::registry::hold(here.world) {
            let reader = Reader {
                queue: here,
                slot: index,
                id: entry.id,
            };
            channel::disarm(&world, endpoint, reader);
        }
    }
}
