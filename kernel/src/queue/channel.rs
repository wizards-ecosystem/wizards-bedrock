use native_abi::Rights;
use native_abi::limits::{MESSAGE_BYTES, MESSAGE_HANDLES};
use native_abi::message::{
    BYTE_CAPACITY, BYTES, BYTES_AT, ENTRIES_AT, ENTRY_BYTES, FLAGS_AT, HANDLE_CAPACITY, HANDLES,
    REPLY, VERSION, VERSION_AT,
};
use native_abi::ring::{Completion, DATA_BYTES, Submission, status};
use wost::mm::pool;

use super::{Pending, PendingKind, Process, Queue, QueueRef, answer, refuse, unresolved};
use crate::arena::Local;
use crate::channel::{
    self, ArmRefusal, Endpoint, Head, NONE_MOVED, NONE_RECEIVED, RIGHTS, Reader, Received,
    WriteRefusal,
};
use crate::handle::SlotReservation;
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

fn header(queue: &Queue, at: u64) -> Option<[u8; HEADER]> {
    let mut header = [0; HEADER];
    (at.is_multiple_of(8) && read_bytes(queue, at, &mut header)).then_some(header)
}

fn endpoint(
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

pub(super) fn write(
    queue: &Queue,
    mut process: Process<'_, '_>,
    submission: Submission,
) -> Completion {
    let token = submission.token;
    let Some(header) = header(queue, submission.at) else {
        return refuse(token, status::BAD_BUFFER);
    };
    if word(&header, VERSION_AT) != VERSION || word(&header, FLAGS_AT) != 0 {
        return refuse(token, status::DENIED);
    }
    let (length, count) = (word(&header, BYTES), word(&header, HANDLES));
    if length > MESSAGE_BYTES || count > MESSAGE_HANDLES {
        return refuse(token, status::LIMIT);
    }
    let (length, count) = (length as usize, count as usize);
    let mut entries = [0; MESSAGE_HANDLES as usize * ENTRY_BYTES];
    let mut bytes = [0; MESSAGE_BYTES as usize];
    let entries = &mut entries[..count * ENTRY_BYTES];
    let bytes = &mut bytes[..length];
    if !read_bytes(queue, submission.at + HEADER as u64, entries)
        || !read_bytes(queue, submission.at + BYTES_AT as u64, bytes)
    {
        return refuse(token, status::BAD_BUFFER);
    }
    let mut named = [(0, Rights::NONE); MESSAGE_HANDLES as usize];
    for (slot, entry) in named.iter_mut().zip(entries.chunks_exact(ENTRY_BYTES)) {
        let mut handle = [0; 8];
        handle.copy_from_slice(&entry[..8]);
        let Some(rights) = Rights::from_bits(word(entry, 8)).filter(|_| word(entry, 12) == 0)
        else {
            return refuse(token, status::DENIED);
        };
        *slot = (u64::from_le_bytes(handle), rights);
    }
    let named = &named[..count];
    let from = match endpoint(&mut process, submission.handle, Rights::WRITE) {
        Ok(endpoint) => endpoint,
        Err(code) => return refuse(token, code),
    };
    let mut moved = NONE_MOVED;
    if process
        .table
        .take(process.context, named, &mut moved)
        .is_err()
    {
        return refuse(token, status::BAD_HANDLE);
    }
    match channel::write(process.world, from, bytes, &mut moved, count) {
        Ok(reader) => {
            process.table.erase(process.context, named);
            if let Some(reader) = reader {
                reader.queue.ready(reader);
            }
            answer(token, 0)
        }
        Err(refusal) => {
            process.table.restore(process.context, named);
            refuse(
                token,
                match refusal {
                    WriteRefusal::PeerClosed => status::PEER_CLOSED,
                    WriteRefusal::Full => status::FULL,
                    WriteRefusal::Denied => status::DENIED,
                },
            )
        }
    }
}

// The room a reader declared, capped at what one message can hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Room {
    bytes: u16,
    handles: u16,
}

fn room(queue: &Queue, at: u64) -> Result<Room, u32> {
    let Some(header) = header(queue, at) else {
        return Err(status::BAD_BUFFER);
    };
    if word(&header, VERSION_AT) != VERSION || word(&header, FLAGS_AT) != 0 {
        return Err(status::DENIED);
    }
    let bytes = word(&header, BYTE_CAPACITY).min(MESSAGE_BYTES);
    let handles = word(&header, HANDLE_CAPACITY).min(MESSAGE_HANDLES);
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
    if let Some(completion) = attempt(queue, &mut process, token, submission.handle, submission.at)
    {
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
    loop {
        if let Some(completion) = attempt(queue, &mut process, entry.token, handle, at) {
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

// `None` when the inbox is empty and the peer still open, so the read must wait.
fn attempt(
    queue: &Queue,
    process: &mut Process<'_, '_>,
    token: u32,
    handle: u64,
    at: u64,
) -> Option<Completion> {
    let room = match room(queue, at) {
        Ok(room) => room,
        Err(code) => return Some(refuse(token, code)),
    };
    let endpoint = match endpoint(process, handle, Rights::READ) {
        Ok(endpoint) => endpoint,
        Err(code) => return Some(refuse(token, code)),
    };
    loop {
        let (bytes, handles, foreign) = match channel::head(process.world, endpoint) {
            Head::Empty => return None,
            Head::PeerClosed => return Some(refuse(token, status::PEER_CLOSED)),
            Head::Message {
                bytes,
                handles,
                foreign,
            } => (bytes, handles, foreign),
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
        let mut slots: [Option<SlotReservation>; MESSAGE_HANDLES as usize] =
            [const { None }; MESSAGE_HANDLES as usize];
        for slot in &mut slots[..usize::from(handles)] {
            let Ok(reserved) = process.table_ref.reserve_slot() else {
                return Some(refuse(token, status::NO_HANDLE_SLOT));
            };
            *slot = Some(reserved);
        }
        let mut payload = [0; MESSAGE_BYTES as usize];
        let mut received = NONE_RECEIVED;
        let Some((bytes, handles)) = channel::take(
            process.world,
            endpoint,
            (room.bytes, handles, foreign),
            &mut payload,
            &mut received,
        ) else {
            continue;
        };
        let installed = install(process, slots, &mut received, earmark.as_mut());
        if !deliver(
            queue,
            at,
            &payload[..usize::from(bytes)],
            &installed[..usize::from(handles)],
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

fn install(
    process: &mut Process<'_, '_>,
    slots: [Option<SlotReservation>; MESSAGE_HANDLES as usize],
    received: &mut Received,
    mut earmark: Option<&mut pool::Earmark>,
) -> [(u64, Rights); MESSAGE_HANDLES as usize] {
    let mut installed = [(0, Rights::NONE); MESSAGE_HANDLES as usize];
    for ((slot, carried), out) in slots
        .into_iter()
        .zip(received.iter_mut())
        .zip(&mut installed)
    {
        let (Some(slot), Some(carried)) = (slot, carried.take()) else {
            continue;
        };
        let object = channel::arrive(earmark.as_deref_mut(), process.world, carried);
        let rights = carried.rights;
        match slot.install(process.context, object, rights, rights) {
            Ok(handle) => *out = (handle, rights),
            Err(_) => object.release(process.world),
        }
    }
    installed
}

fn deliver(queue: &Queue, at: u64, payload: &[u8], handles: &[(u64, Rights)]) -> bool {
    let Ok(at) = usize::try_from(at) else {
        return false;
    };
    let mut header = [0u8; HEADER];
    header[BYTES..BYTES + 4].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    header[HANDLES..HANDLES + 4].copy_from_slice(&(handles.len() as u32).to_le_bytes());
    let mut written = queue.write_data(at + BYTES, &header[BYTES..HANDLES + 4])
        && queue.write_data(at + REPLY, &0u64.to_le_bytes())
        && queue.write_data(at + BYTES_AT, payload);
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
    fn park(&self, token: u32, kind: PendingKind) -> Option<(usize, Pending)> {
        let mut pending = self.pending.lock_leaf();
        let (index, entry) = pending.insert(token, kind)?;
        let claimed = Pending {
            settling: true,
            ..entry
        };
        pending.slots[index] = Some(claimed);
        Some((index, claimed))
    }

    fn unpark(&self, index: usize, entry: Pending) {
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
