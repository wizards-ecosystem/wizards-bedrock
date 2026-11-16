use alloc::sync::Arc;

use native_abi::Rights;
use native_abi::ring::{Completion, Opcode, Submission, status};

use super::clocks::spins_until;
use super::runs_until;
use crate::handle::HandleTable;
use crate::queue::Queue;
use crate::vfs;

// What `xtask/src/image/corpus.rs` puts on the volume.
const SMALL_NAME: &[u8] = b"HELLO.TXT";
const SMALL_CONTENT: &[u8] = b"the wizard's bedrock reads a disk\n";
const DIRECTORY_NAME: &[u8] = b"DATA";

pub(super) fn table(quota: usize) -> Option<Arc<HandleTable>> {
    HandleTable::new(quota).ok().map(Arc::new)
}

pub(super) fn post(queue: &Queue, from: usize, submissions: &[Submission]) {
    for (index, submission) in (from..).zip(submissions) {
        queue.post_submission_for_test(index, *submission);
    }
    queue.publish_tail_for_test((from + submissions.len()) as u32);
}

fn served_by_the_worker(queue: &Arc<Queue>, table: &Arc<HandleTable>, count: usize) -> bool {
    let before = queue.served_count();
    queue.serve(table) == 0 && runs_until(5_000, || queue.served_count() == before + count)
}

pub(super) fn every_submission_gets_exactly_one_completion() -> bool {
    let (Some(queue), Some(runaway), Some(partial), Some(table)) = (
        Queue::new(crate::timepage::root()),
        Queue::new(crate::timepage::root()),
        Queue::new(crate::timepage::root()),
        table(8),
    ) else {
        return false;
    };

    post(
        &queue,
        0,
        &[
            Submission::simple(Opcode::Echo, 0x1111, 0xDEAD_BEEF),
            Submission::simple(Opcode::Uptime, 0x2222, 0),
            Submission::simple(Opcode::Unknown(0xFFFF), 0x3333, 0),
        ],
    );
    let once = queue.serve(&table) == 3 && queue.serve(&table) == 0;
    let accounted = queue.outstanding() == 0 && queue.is_consistent();

    let echo = queue.read_completion(0);
    let uptime = queue.read_completion(1);
    let refused = queue.read_completion(2);
    let matched = echo.token == 0x1111
        && echo.status == status::OK
        && echo.value == 0xDEAD_BEEF
        && uptime.token == 0x2222
        && uptime.status == status::OK
        && uptime.value > 0
        && uptime.value <= crate::time::now_ns()
        && refused.token == 0x3333
        && refused.status == status::UNKNOWN_OPCODE;

    let echoes = [
        Submission::simple(Opcode::Echo, 0x5150, 7),
        Submission::simple(Opcode::Echo, 0x5151, 7),
        Submission::simple(Opcode::Echo, 0x5152, 7),
    ];
    // Three written under a tail claiming four billion: unbounded, the kernel serves all three.
    post(&runaway, 0, &echoes);
    runaway.publish_tail_for_test(u32::MAX);
    let runaway_bounded = runaway.serve(&table) == 0 && runaway.outstanding() == 0;

    post(&partial, 0, &echoes);
    partial.publish_tail_for_test(1);
    let only_published = partial.serve(&table) == 1
        && partial.read_completion(0).token == 0x5150
        && partial.outstanding() == 0
        && partial.is_consistent();

    once && accounted
        && matched
        && runaway_bounded
        && only_published
        && never_reaped_completions_are_not_overwritten(&table)
}

// The program never moves its completion head, so the third round must wait rather than overwrite.
fn never_reaped_completions_are_not_overwritten(table: &Arc<HandleTable>) -> bool {
    const ROUND: usize = 8;

    let Some(queue) = Queue::new(crate::timepage::root()) else {
        return false;
    };
    let round = |first: usize| -> [Submission; ROUND] {
        core::array::from_fn(|index| {
            Submission::simple(Opcode::Echo, (first + index) as u32, (first + index) as u64)
        })
    };
    post(&queue, 0, &round(0));
    let first = queue.serve(table) == ROUND;
    post(&queue, ROUND, &round(ROUND));
    let second = queue.serve(table) == ROUND;
    post(&queue, 2 * ROUND, &round(2 * ROUND));
    let refused = queue.serve(table) == 0;

    let kept = (0..2 * ROUND).all(|slot| {
        let completion = queue.read_completion(slot);
        completion.token == slot as u32 && completion.value == slot as u64
    });
    first && second && refused && kept && queue.outstanding() == 0 && queue.is_consistent()
}

pub(super) fn a_word_that_does_not_fit_is_refused_rather_than_truncated() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let (Some(queue), Some(table)) = (Queue::new(crate::timepage::root()), table(8)) else {
        return false;
    };
    let full = Rights::UNIVERSAL;
    let Ok(source) = table.insert(crate::object::bootstrap_identity(), full) else {
        return false;
    };
    let Some((widened, cancelled)) =
        serve_an_unfit_rights_word_and_token(&queue, &table, source, full)
    else {
        return false;
    };
    let Some(file) = open_the_small_file_in(&queue, &table, 3) else {
        return false;
    };
    let Some((at_max, past)) = read_past_every_file(&queue, &table, file) else {
        return false;
    };

    let mut untouched = [0u8; 32];
    if !queue.read_data_for_test(512, &mut untouched) {
        return false;
    }

    widened.status == status::DENIED
        // The timer is still in flight: a token that does not fit names no operation.
        && cancelled.status == status::NOT_FOUND
        && queue.in_flight() == 1
        && at_max.status == status::OK
        && at_max.value == 0
        && past.status == status::OK
        && past.value == 0
        && untouched == [0u8; 32]
}

fn serve_an_unfit_rights_word_and_token(
    queue: &Arc<Queue>,
    table: &Arc<HandleTable>,
    source: u32,
    full: Rights,
) -> Option<(Completion, Completion)> {
    // A valid low half under an invalid high half: a cast reads the low half, `try_from` refuses.
    post(
        queue,
        0,
        &[
            Submission::about(
                Opcode::Duplicate,
                0xC001,
                source,
                (0xFFFF_FFFFu64 << 32) | u64::from(full.bits()),
            ),
            Submission::simple(Opcode::Timer, 0xDEAD, 3_600_000_000_000),
        ],
    );
    if queue.serve(table) != 1 {
        return None;
    }
    let widened = queue.read_completion(0);

    post(
        queue,
        2,
        &[Submission::simple(
            Opcode::Cancel,
            0xC002,
            (1u64 << 32) | 0xDEAD,
        )],
    );
    if queue.serve(table) != 1 {
        return None;
    }
    Some((widened, queue.read_completion(1)))
}

fn read_past_every_file(
    queue: &Arc<Queue>,
    table: &Arc<HandleTable>,
    file: u32,
) -> Option<(Completion, Completion)> {
    post(
        queue,
        4,
        &[
            Submission::about(Opcode::Read, 0xC003, file, 512)
                .offset(u64::from(u32::MAX))
                .length(16),
            Submission::about(Opcode::Read, 0xC004, file, 528)
                .offset(1u64 << 32)
                .length(16),
        ],
    );
    served_by_the_worker(queue, table, 2)
        .then(|| (queue.read_completion(3), queue.read_completion(4)))
}

fn open_the_small_file_in(
    queue: &Arc<Queue>,
    table: &Arc<HandleTable>,
    slot: usize,
) -> Option<u32> {
    let object = crate::object::directory(vfs::root()?);
    let directory = table
        .insert(object, Rights::UNIVERSAL.union(Rights::TRAVERSE))
        .ok()?;
    if !queue.write_data_for_test(0, SMALL_NAME) {
        return None;
    }
    post(
        queue,
        slot,
        &[Submission::about(Opcode::OpenAt, 0x0F11, directory, 0).length(SMALL_NAME.len() as u32)],
    );
    if !served_by_the_worker(queue, table, 1) {
        return None;
    }
    let completion = queue.read_completion(2);
    (completion.status == status::OK).then_some(completion.value as u32)
}

pub(super) fn opening_a_node_does_not_restore_universal_rights() -> bool {
    const DIRECTORY_AT: u64 = 64;
    const READ_AT: u64 = 128;

    let (Some(queue), Some(table), Some(root)) =
        (Queue::new(crate::timepage::root()), table(8), vfs::root())
    else {
        return false;
    };
    let (Ok(walkable), Ok(universal)) = (
        table.insert(crate::object::directory(root), Rights::TRAVERSE),
        table.insert(
            crate::object::directory(root),
            Rights::UNIVERSAL
                .union(Rights::TRAVERSE)
                .union(Rights::ENUMERATE),
        ),
    ) else {
        return false;
    };
    if !queue.write_data_for_test(0, SMALL_NAME)
        || !queue.write_data_for_test(DIRECTORY_AT as usize, DIRECTORY_NAME)
    {
        return false;
    }
    post(
        &queue,
        0,
        &[
            Submission::about(Opcode::OpenAt, 0xA771, walkable, 0).length(SMALL_NAME.len() as u32),
            Submission::about(Opcode::OpenAt, 0xA772, walkable, DIRECTORY_AT)
                .length(DIRECTORY_NAME.len() as u32),
            Submission::about(Opcode::OpenAt, 0xA773, universal, 0).length(SMALL_NAME.len() as u32),
        ],
    );
    if !served_by_the_worker(&queue, &table, 3) {
        return false;
    }

    let opened = [0, 1, 2].map(|slot| queue.read_completion(slot));
    if opened.iter().any(|open| open.status != status::OK) {
        return false;
    }
    let [file, subdirectory, universal_file] = opened.map(|open| open.value as u32);
    let Ok((file_object, file_rights)) = table.resolve(file) else {
        return false;
    };
    let Ok((directory, directory_rights)) = table.resolve(subdirectory) else {
        return false;
    };
    let Ok((_, universal_rights)) = table.resolve(universal_file) else {
        return false;
    };
    let narrowed = file_object.as_file().is_some()
        && file_rights == Rights::READ
        && directory.as_directory().is_some()
        && directory_rights == Rights::TRAVERSE
        // Read only: there is no write path, and a right that grants nothing is a disguised call.
        && universal_rights == Rights::UNIVERSAL.union(Rights::READ)
        && table.live() == 5
        && table.promised() == 0;

    post(
        &queue,
        3,
        &[Submission::about(Opcode::Read, 0xD200, file, READ_AT).length(16)],
    );
    if !served_by_the_worker(&queue, &table, 1) {
        return false;
    }
    let read = queue.read_completion(3);
    let mut bytes = [0u8; 16];
    let copied = queue.read_data_for_test(READ_AT as usize, &mut bytes);

    narrowed
        && read.token == 0xD200
        && read.status == status::OK
        && read.value == 16
        && copied
        && bytes == SMALL_CONTENT[..16]
        && queue.outstanding() == 0
        && queue.abandon() == 0
}

pub(super) fn a_ring_operation_cannot_reach_past_its_rights_or_its_pages() -> bool {
    let (Some(queue), Some(table), Some(root)) =
        (Queue::new(crate::timepage::root()), table(8), vfs::root())
    else {
        return false;
    };
    let Some((unwalkable, walkable, unreadable, readable)) = reach_fixture(&queue, &table, root)
    else {
        return false;
    };
    let Some(refused) = opens_are_refused(&queue, &table, unwalkable, walkable) else {
        return false;
    };

    post(
        &queue,
        4,
        &[
            Submission::about(Opcode::Read, 0xD5, unreadable, 0).length(16),
            Submission::about(Opcode::Read, 0xD6, unreadable, 4090).length(16),
            Submission::about(Opcode::Read, 0xD7, readable, 4090).length(16),
        ],
    );
    let served = queue.serve(&table) == 3;
    let denied = queue.read_completion(4).status == status::DENIED
        && queue.read_completion(5).status == status::DENIED
        && queue.read_completion(6).status == status::BAD_BUFFER;

    // Every refusal gave its promised slot back.
    refused && served && denied && table.live() == 4 && table.promised() == 0
}

fn reach_fixture(
    queue: &Arc<Queue>,
    table: &Arc<HandleTable>,
    root: vfs::Directory,
) -> Option<(u32, u32, u32, u32)> {
    let unwalkable = table
        .insert(
            crate::object::directory(root),
            Rights::UNIVERSAL.union(Rights::ENUMERATE),
        )
        .ok()?;
    let walkable = table
        .insert(
            crate::object::directory(root),
            Rights::UNIVERSAL.union(Rights::TRAVERSE),
        )
        .ok()?;

    if !queue.write_data_for_test(0, SMALL_NAME) || !queue.write_data_for_test(256, b"..") {
        return None;
    }

    let vfs::Node::File(file) = vfs::resolve(root, SMALL_NAME).ok()? else {
        return None;
    };
    let unreadable = table
        .insert(crate::object::file(file), Rights::UNIVERSAL)
        .ok()?;
    let readable = table
        .insert(
            crate::object::file(file),
            Rights::UNIVERSAL.union(Rights::READ),
        )
        .ok()?;
    Some((unwalkable, walkable, unreadable, readable))
}

fn opens_are_refused(
    queue: &Arc<Queue>,
    table: &Arc<HandleTable>,
    unwalkable: u32,
    walkable: u32,
) -> Option<bool> {
    let asked = [
        (
            0xD1u32,
            unwalkable,
            0u64,
            SMALL_NAME.len() as u32,
            status::DENIED,
        ),
        (0xD2, walkable, 256, 2, status::BAD_PATH),
        (0xD3, walkable, 8192, 8, status::BAD_BUFFER),
        (0xD4, walkable, 0, 0, status::BAD_BUFFER),
    ];
    post(
        queue,
        0,
        &asked.map(|(token, handle, at, length, _)| {
            Submission::about(Opcode::OpenAt, token, handle, at).length(length)
        }),
    );
    let served_before = queue.served_count();
    if queue.serve(table) != asked.len() - 1
        || !runs_until(5_000, || {
            queue.served_count() == served_before + asked.len()
        })
    {
        return None;
    }

    Some(asked.iter().all(|(token, _, _, _, status)| {
        (0..asked.len()).any(|slot| {
            let completion = queue.read_completion(slot);
            completion.token == *token && completion.status == *status
        })
    }))
}

pub(super) fn an_operation_completes_after_its_deadline_and_only_once() -> bool {
    const DELAY_NS: u64 = 2_000_000;
    const HOUR_NS: u64 = 3_600_000_000_000;

    if !crate::time::is_calibrated() {
        return false;
    }
    let (Some(queue), Some(table)) = (Queue::new(crate::timepage::root()), table(8)) else {
        return false;
    };

    let submitted_at = crate::timepage::root().now_ns();
    post(
        &queue,
        0,
        &[
            Submission::simple(Opcode::Timer, 0x7117, DELAY_NS),
            Submission::simple(Opcode::Timer, 0xDEAD, HOUR_NS),
            Submission::simple(Opcode::Timer, 0xA1, HOUR_NS),
        ],
    );
    let deferred = queue.serve(&table) == 0 && queue.in_flight() == 3;
    let Some(deadline) = queue.earliest_deadline() else {
        return false;
    };
    // The deadline is two milliseconds out on a running clock: one this does not reach is wrong.
    if !spins_until(1_000_000, || crate::time::now_ns() >= deadline) {
        return false;
    }

    let early = queue.expire(deadline - 1) == 0 && queue.in_flight() == 3;
    let due = queue.expire(crate::time::now_ns()) == 1;
    let timer = queue.read_completion(0);
    let once = queue.expire(crate::time::now_ns()) == 0
        && timer.token == 0x7117
        && timer.status == status::OK
        && timer.value >= submitted_at.saturating_add(DELAY_NS);

    post(
        &queue,
        3,
        &[Submission::simple(Opcode::Cancel, 0xC0DE, 0xDEAD)],
    );
    let cancelled = queue.serve(&table) == 1;
    let target = queue.read_completion(1);
    let canceller = queue.read_completion(2);
    let separate = cancelled
        && target.token == 0xDEAD
        && target.status == status::CANCELLED
        && canceller.token == 0xC0DE
        && canceller.status == status::OK
        && canceller.value == 0xDEAD;

    post(
        &queue,
        4,
        &[Submission::simple(Opcode::Cancel, 0xC0DF, 0xDEAD)],
    );
    let again = queue.serve(&table) == 1 && queue.read_completion(3).status == status::NOT_FOUND;

    // The process-teardown path ends what is left, once.
    let abandoned = queue.abandon() == 1;
    let ended = queue.read_completion(4);
    let released = abandoned
        && ended.token == 0xA1
        && ended.status == status::CANCELLED
        && queue.is_quiet()
        && queue.in_flight() == 0
        && queue.outstanding() == 0
        && queue.is_consistent()
        && queue.abandon() == 0;

    deferred && early && due && once && separate && again && released
}
