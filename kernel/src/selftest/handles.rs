use alloc::sync::Arc;

use handle_space::Rights;

use crate::handle::{HandleError, HandleTable};
use crate::object::{self, ObjectType};
use crate::queue;

pub(super) fn a_completion_carries_a_handle_with_narrowed_rights() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let held = crate::object::bootstrap_identity();
    let table = shared_handle_table_or_false!(8);

    let offered = Rights::UNIVERSAL;
    let Ok(source) = table.insert(held, offered) else {
        return false;
    };

    let declared = Rights::INSPECT;
    queue.post_submission_for_test(
        0,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0x5150,
            source,
            u64::from(declared.bits()),
        ),
    );
    queue.publish_tail_for_test(1);

    if queue.serve(&table) != 1 {
        return false;
    }
    let completion = queue.read_completion(0);
    if completion.token != 0x5150 || completion.status != queue::STATUS_OK {
        return false;
    }

    let arrived = completion.value as u32;
    let Ok((_, granted)) = table.resolve(arrived) else {
        return false;
    };

    granted == declared
        && !granted.contains(Rights::DUPLICATE)
        && arrived != source
        && table.live() == 2
        // The promise became a handle: a table charged twice for one runs
        // out at half its quota and nothing else would say so.
        && table.promised() == 0
}

pub(super) fn a_completion_cannot_widen_what_was_offered() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let held = crate::object::bootstrap_identity();
    let table = shared_handle_table_or_false!(8);

    // Held without WAIT or TRANSFER, so asking for all universal rights widens.
    let offered = Rights::DUPLICATE.union(Rights::INSPECT);
    let Ok(source) = table.insert(held, offered) else {
        return false;
    };

    let asked = Rights::UNIVERSAL;
    queue.post_submission_for_test(
        0,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0x0BAD,
            source,
            u64::from(asked.bits()),
        ),
    );
    queue.publish_tail_for_test(1);

    if queue.serve(&table) != 1 {
        return false;
    }
    let completion = queue.read_completion(0);

    completion.token == 0x0BAD
        && completion.status == queue::STATUS_DENIED
        && completion.value == 0
        && table.live() == 1
        && table.promised() == 0
}

pub(super) fn a_refused_handle_operation_returns_its_promise() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let duplicable = crate::object::bootstrap_identity();
    let sealed = crate::object::bootstrap_identity();
    let attenuated = crate::object::bootstrap_identity();

    // Room to spare: a refusal must be reached *after* a promise is taken, and
    // a full table refuses before there is one.
    let table = shared_handle_table_or_false!(8);
    let full = Rights::UNIVERSAL;
    let Ok(source) = table.insert(duplicable, full) else {
        return false;
    };
    // Held without DUPLICATE, which is what makes copying it a refusal rather
    // than a mistake.
    let Ok(undupable) = table.insert(sealed, Rights::INSPECT) else {
        return false;
    };
    let narrow = Rights::DUPLICATE.union(Rights::INSPECT);
    let Ok(attenuation_source) = table.insert(attenuated, narrow) else {
        return false;
    };
    let attenuation_request = narrow.union(Rights::WAIT);

    let asked = [
        // Refused at the lookup: a well-formed value naming nothing of ours.
        (
            0xF001u32,
            source ^ 0x40,
            u64::from(full.bits()),
            queue::STATUS_BAD_HANDLE,
        ),
        // Refused at the right: held, but not passable.
        (
            0xF002,
            undupable,
            u64::from(Rights::INSPECT.bits()),
            queue::STATUS_DENIED,
        ),
        // Refused at the attenuation: asking for more than is held.
        (
            0xF003,
            attenuation_source,
            u64::from(attenuation_request.bits()),
            queue::STATUS_DENIED,
        ),
    ];

    let (_, attenuation_handle, attenuation_bits, _) = asked[2];
    let Ok((held, held_rights)) = table.resolve(attenuation_handle) else {
        return false;
    };
    let Some(requested) = u32::try_from(attenuation_bits)
        .ok()
        .and_then(Rights::from_bits)
    else {
        return false;
    };
    if !held_rights.contains(Rights::DUPLICATE)
        || held_rights.contains(Rights::WAIT)
        || !requested.contains(Rights::WAIT)
        || !requested.are_meaningful_for(held.kind().per_type_rights())
        || held_rights.attenuate(requested).is_some()
        || table.live() != 3
        || table.promised() != 0
    {
        return false;
    }

    for (slot, (token, handle, rights, _)) in asked.iter().enumerate() {
        queue.post_submission_for_test(
            slot,
            queue::Submission::about(queue::Opcode::Duplicate, *token, *handle, *rights),
        );
    }
    queue.publish_tail_for_test(asked.len() as u32);

    if queue.serve(&table) != asked.len() {
        return false;
    }

    let mut refused = true;
    for (slot, (token, _, _, status)) in asked.iter().enumerate() {
        let completion = queue.read_completion(slot);
        refused &=
            completion.token == *token && completion.status == *status && completion.value == 0;
    }

    // Three promises taken, three given back: a leak is invisible in the
    // completions and shows later as a table full of nothing.
    refused && table.live() == 3 && table.promised() == 0
}

pub(super) fn a_full_table_refuses_a_handle_operation_at_submission() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let held = crate::object::bootstrap_identity();

    // Room for exactly one, which the source takes.
    let table = shared_handle_table_or_false!(1);
    let full = Rights::UNIVERSAL;
    let Ok(source) = table.insert(held, full) else {
        return false;
    };

    queue.post_submission_for_test(
        0,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0xFEED,
            source,
            u64::from(full.bits()),
        ),
    );
    // Neither malformed rights nor a malformed path may be inspected first,
    // or the result-producing operations have two admission orders (D-151).
    queue.post_submission_for_test(
        1,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0xFEE1,
            source,
            0xFFFF_FFFF_0000_0000,
        ),
    );
    queue.post_submission_for_test(
        2,
        queue::Queue::about_at(queue::Opcode::OpenAt, 0xFEE2, source, 0, u64::MAX, 1),
    );
    queue.publish_tail_for_test(3);

    if queue.serve(&table) != 3 {
        return false;
    }
    let duplicate = queue.read_completion(0);
    let malformed_duplicate = queue.read_completion(1);
    let open = queue.read_completion(2);

    duplicate.token == 0xFEED
        && duplicate.status == queue::STATUS_NO_HANDLE_SLOT
        && duplicate.value == 0
        && malformed_duplicate.token == 0xFEE1
        && malformed_duplicate.status == queue::STATUS_NO_HANDLE_SLOT
        && malformed_duplicate.value == 0
        && open.token == 0xFEE2
        && open.status == queue::STATUS_NO_HANDLE_SLOT
        && open.value == 0
        && table.live() == 1
        && table.promised() == 0
}

pub(super) fn a_handle_reveals_no_foreign_occupancy() -> bool {
    let mine = handle_table_or_false!(4);
    let theirs = handle_table_or_false!(4);
    let object = object::bootstrap_identity();
    let Ok(handle) = mine.insert(object, Rights::UNIVERSAL) else {
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

pub(super) fn table_identities_never_wrap_or_repeat() -> bool {
    let baseline = HandleTable::arena_usage_for_test();
    HandleTable::with_next_identity_for_test(u64::MAX - 1, || {
        let Ok(last) = HandleTable::new(1) else {
            return false;
        };
        let object = object::bootstrap_identity();
        let Ok(handle) = last.insert(object, Rights::UNIVERSAL) else {
            return false;
        };
        let live_commitment = HandleTable::arena_usage_for_test();

        let first = HandleTable::new(1);
        let second = HandleTable::new(1);
        let refused_twice = matches!(&first, Err(HandleError::IdentityExhausted))
            && matches!(&second, Err(HandleError::IdentityExhausted));
        let duplicate_authority = first
            .as_ref()
            .ok()
            .is_some_and(|table| table.resolve(handle).is_ok())
            || second
                .as_ref()
                .ok()
                .is_some_and(|table| table.resolve(handle).is_ok());
        let refusal_changed_nothing = HandleTable::arena_usage_for_test() == live_commitment;
        let last_stayed_live = last.identity_for_test() == u64::MAX - 1
            && last.kind_of(handle) == Ok(ObjectType::Channel);

        drop(first);
        drop(second);
        drop(last);

        refused_twice
            && !duplicate_authority
            && refusal_changed_nothing
            && last_stayed_live
            && HandleTable::arena_usage_for_test() == baseline
    })
}

pub(super) fn a_table_cannot_exceed_its_quota() -> bool {
    const QUOTA: usize = 3;
    let table = handle_table_or_false!(QUOTA);
    for _ in 0..QUOTA {
        let object = object::bootstrap_identity();
        if table.insert(object, Rights::UNIVERSAL).is_err() {
            return false;
        }
    }

    let extra = object::bootstrap_identity();
    let refused = table.insert(extra, Rights::UNIVERSAL) == Err(HandleError::QuotaExhausted);
    refused && table.live() == QUOTA
}

pub(super) fn the_handle_arena_backs_a_tables_whole_quota() -> bool {
    let (committed_before, capacity_before) = HandleTable::arena_backing_for_test();
    let Some(spare) = capacity_before.checked_sub(committed_before) else {
        return false;
    };
    let Some(quota) = spare.checked_add(1) else {
        return false;
    };
    if quota > (handle_space::SLOTS as usize).saturating_sub(committed_before) {
        return false;
    }

    let Ok(table) = HandleTable::new(quota) else {
        return false;
    };
    let (committed, capacity) = HandleTable::arena_backing_for_test();
    let backed = committed == committed_before + quota
        && capacity > capacity_before
        && capacity >= committed;

    for _ in 0..quota {
        let object = object::bootstrap_identity();
        if table.insert(object, Rights::UNIVERSAL).is_err() {
            return false;
        }
    }
    let (filled_commitment, filled_capacity) = HandleTable::arena_backing_for_test();
    let filled_without_growth =
        table.live() == quota && filled_commitment == committed && filled_capacity == capacity;

    drop(table);
    let (committed_after, capacity_after) = HandleTable::arena_backing_for_test();
    let returned = committed_after == committed_before && capacity_after == filled_capacity;

    backed && filled_without_growth && returned
}

pub(super) fn an_object_lives_exactly_as_long_as_its_handles() -> bool {
    let first = handle_table_or_false!(1);
    let second = handle_table_or_false!(1);
    let object = object::bootstrap_identity();
    let Some(watcher) = object.downgrade_for_test() else {
        return false;
    };
    if first.insert(object.clone(), Rights::UNIVERSAL).is_err()
        || second.insert(object, Rights::UNIVERSAL).is_err()
    {
        return false;
    }
    let alive = watcher.upgrade().is_some();
    drop(first);
    let still_alive = watcher.upgrade().is_some();
    drop(second);
    let gone = watcher.upgrade().is_none();

    alive && still_alive && gone
}

pub(super) fn a_dead_table_releases_its_objects() -> bool {
    let object = object::bootstrap_identity();
    let Some(watcher) = object.downgrade_for_test() else {
        return false;
    };
    {
        let table = handle_table_or_false!(4);
        if table.insert(object, Rights::UNIVERSAL).is_err() {
            return false;
        }
        if watcher.upgrade().is_none() {
            return false;
        }
    }
    watcher.upgrade().is_none()
}

pub(super) fn filling_the_arena_never_repeats_a_slot() -> bool {
    /// Enough to wrap the hint past the end and force a grow or two.
    const HANDLES: usize = 24;

    let table = handle_table_or_false!(HANDLES);
    let mut handles = [0u32; HANDLES];

    for slot in &mut handles {
        let object = object::bootstrap_identity();
        let Ok(handle) = table.insert(object, Rights::UNIVERSAL) else {
            return false;
        };
        *slot = handle;
    }

    // Distinct values. Two claims of one slot would give the same index and
    // the same generation, so the handles themselves would collide.
    let mut distinct = true;
    for (index, handle) in handles.iter().enumerate() {
        for earlier in &handles[..index] {
            distinct &= earlier != handle;
        }
    }

    // And every one still resolves — an overwrite leaves the loser naming a
    // slot that is stale in its table rather than the object it was given.
    let all_live = handles
        .iter()
        .all(|handle| table.kind_of(*handle) == Ok(ObjectType::Channel));

    let counted = table.live() == HANDLES;

    distinct && all_live && counted
}
