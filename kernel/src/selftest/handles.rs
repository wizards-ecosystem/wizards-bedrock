use native_abi::Rights;
use native_abi::ring::{Opcode, Submission, status};

use super::rings::{post, table};
use crate::handle::{HandleError, HandleTable};
use crate::object;
use crate::queue::Queue;

pub(super) fn a_completion_cannot_widen_what_was_offered() -> bool {
    let (Some(queue), Some(table), Some(full_table)) = (Queue::new(), table(8), table(1)) else {
        return false;
    };
    let full = Rights::UNIVERSAL;
    let limited = Rights::DUPLICATE.union(Rights::INSPECT);
    let (Ok(source), Ok(narrow), Ok(sealed), Ok(only)) = (
        table.insert(object::bootstrap_identity(), full),
        table.insert(object::bootstrap_identity(), limited),
        table.insert(object::bootstrap_identity(), Rights::INSPECT),
        full_table.insert(object::bootstrap_identity(), full),
    ) else {
        return false;
    };
    let bits = |rights: Rights| u64::from(rights.bits());

    post(
        &queue,
        0,
        &[
            Submission::about(Opcode::Duplicate, 0xF001, source ^ 0x40, bits(full)),
            Submission::about(Opcode::Duplicate, 0x5150, source, bits(Rights::INSPECT)),
            Submission::about(Opcode::Duplicate, 0x0BAD, narrow, bits(full)),
            Submission::about(Opcode::Duplicate, 0xF002, sealed, bits(Rights::INSPECT)),
        ],
    );
    if queue.serve(&table) != 4 {
        return false;
    }
    let narrowed = queue.read_completion(1);
    let granted = table
        .resolve(narrowed.value as u32)
        .is_ok_and(|(_, rights)| rights == Rights::INSPECT);
    let refused = [
        (0, 0xF001, status::BAD_HANDLE),
        (2, 0x0BAD, status::DENIED),
        (3, 0xF002, status::DENIED),
    ]
    .into_iter()
    .all(|(slot, token, status)| {
        let completion = queue.read_completion(slot);
        completion.token == token && completion.status == status && completion.value == 0
    });

    post(
        &queue,
        4,
        &[Submission::about(
            Opcode::Duplicate,
            0xFEED,
            only,
            bits(full),
        )],
    );
    let served_full = queue.serve(&full_table) == 1;
    let no_slot = queue.read_completion(4);

    served_full
        && no_slot.token == 0xFEED
        && no_slot.status == status::NO_HANDLE_SLOT
        && no_slot.value == 0
        && narrowed.token == 0x5150
        && narrowed.status == status::OK
        && narrowed.value as u32 != source
        && granted
        && refused
        // Every refusal gave its promise back: a leak shows later as a table full of nothing.
        && table.live() == 4
        && table.promised() == 0
        && full_table.live() == 1
        && full_table.promised() == 0
}

pub(super) fn a_handle_reveals_no_foreign_occupancy() -> bool {
    let (Ok(mine), Ok(theirs)) = (HandleTable::new(4), HandleTable::new(4)) else {
        return false;
    };
    let Ok(handle) = mine.insert(object::bootstrap_identity(), Rights::UNIVERSAL) else {
        return false;
    };
    let Some(slot) = mine.slot_of(handle) else {
        return false;
    };
    let forged = theirs.handle_for(slot);
    let live_resolve = theirs.resolve(forged).err();
    let foreign_changed_nothing =
        mine.resolve(handle).is_ok() && mine.live() == 1 && theirs.live() == 0;

    drop(mine);
    let absent_resolve = theirs.resolve(forged).err();

    live_resolve == Some(HandleError::Stale)
        && live_resolve == absent_resolve
        && foreign_changed_nothing
        && theirs.live() == 0
}
