use native_abi::Rights;
use native_abi::ring::{Completion, Submission, status};
use native_abi::world::{BYTES, VERSION, offset};
use world_tree::{Limits, NodeRef};

use super::{PendingKind, Process, Queue, answer, refuse};
use crate::object::ObjectRef;
use crate::world::{KillRefusal, Killer, Refusal, Request};

const RIGHTS: Rights = Rights::DUPLICATE
    .union(Rights::TRANSFER)
    .union(Rights::INSPECT)
    .union(Rights::SPAWN)
    .union(Rights::KILL);

// The new world is a child of the caller's, and the handle is the only way back to it.
pub(super) fn create(
    queue: &Queue,
    process: Process<'_, '_>,
    submission: Submission,
) -> Completion {
    let token = submission.token;
    let mut bytes = [0; BYTES];
    let bounds = Submission {
        length: BYTES as u32,
        ..submission
    };
    if !submission.at.is_multiple_of(8) || queue.read_data(&bounds, &mut bytes) != Some(BYTES) {
        return refuse(token, status::BAD_BUFFER);
    }
    let word =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    if word(offset::VERSION) != VERSION || word(offset::RESERVED) != 0 {
        return refuse(token, status::DENIED);
    }
    let request = Request {
        per_colour: word(offset::FRAMES_PER_COLOUR),
        mask: u64::from(word(offset::COLOUR_MASK)) | u64::from(word(offset::COLOUR_MASK + 4)) << 32,
        limits: Limits {
            descendants: word(offset::DESCENDANTS),
            threads: word(offset::THREADS),
            handles: word(offset::HANDLES),
        },
    };
    let Ok(reservation) = process.table_ref.reserve_slot() else {
        return refuse(token, status::NO_HANDLE_SLOT);
    };
    let node = match crate::world::create(process.context, process.here.world, request) {
        Ok(node) => node,
        Err(Refusal::NoMemory) => return refuse(token, status::NO_MEMORY),
        Err(Refusal::Limit) => return refuse(token, status::LIMIT),
        Err(Refusal::Dead) => return refuse(token, status::DEAD),
    };
    match reservation.install(process.context, ObjectRef::World(node), RIGHTS, RIGHTS) {
        Ok(handle) => answer(token, handle),
        Err(_) => {
            crate::world::kill(node);
            refuse(token, status::BAD_HANDLE)
        }
    }
}

// Completes once the world is gone and its frames are its heir's; its descendants end on their own.
pub(super) fn kill(
    queue: &Queue,
    mut process: Process<'_, '_>,
    submission: Submission,
) -> Option<Completion> {
    let token = submission.token;
    let node = match resolve(&mut process, submission.handle, Rights::KILL) {
        Ok(node) => node,
        Err(status) => return Some(refuse(token, status)),
    };
    let Some((pending, entry)) = queue.pending.lock_leaf().insert(token, PendingKind::Kill) else {
        return Some(refuse(token, status::NO_ROOM));
    };
    let killer = Killer {
        queue: process.here,
        pending,
        id: entry.id,
    };
    let refused = match crate::world::kill_for(node, killer) {
        Ok(()) => return None,
        Err(KillRefusal::Dead) => status::DEAD,
        Err(KillRefusal::Root) => status::DENIED,
        Err(KillRefusal::Full) => status::NO_ROOM,
    };
    queue.pending.lock_leaf().slots[pending] = None;
    Some(refuse(token, refused))
}

// Handle 0 names the caller's own world.
pub(super) fn inspect(mut process: Process<'_, '_>, submission: Submission) -> Completion {
    let token = submission.token;
    let node = match submission.handle {
        0 => process.here.world,
        handle => match resolve(&mut process, handle, Rights::INSPECT) {
            Ok(node) => node,
            Err(status) => return refuse(token, status),
        },
    };
    match crate::world::inspect(node) {
        Some((reserved, used)) => Completion {
            extra: used as u64,
            ..answer(token, reserved as u64)
        },
        None => refuse(token, status::DEAD),
    }
}

fn resolve(process: &mut Process<'_, '_>, handle: u64, wanted: Rights) -> Result<NodeRef, u32> {
    let Ok((object, rights)) = process.table.resolve(process.context, handle) else {
        return Err(status::BAD_HANDLE);
    };
    let ObjectRef::World(node) = object else {
        return Err(status::DENIED);
    };
    if !rights.contains(wanted) {
        return Err(status::DENIED);
    }
    Ok(node)
}
