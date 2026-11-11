use alloc::sync::Arc;

use handle_space::Rights;
use wost::arch::current::interrupt;
use wost::arch::current::paging::{AddressSpace, MapError};
use x86_paging::{Flags, PAGE_SIZE};

use super::address_spaces::in_reclaimed_space;
use super::clocks::spins_until;
use super::files::{DIRECTORY_NAME, SMALL_CONTENT, SMALL_NAME};
use super::threads::the_clock_is_running;
use super::{awaiting, runs_until};
use crate::handle::{HandleError, HandleTable};
use crate::nvme;
use crate::queue;
use crate::user;
use crate::vfs;
use crate::{info, input, thread, time};

pub(super) fn a_full_ring_admits_again_only_after_userspace_reaps() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let (_, submissions, completions) = queue.shape();

    let mut posted = 0u32;
    while posted < completions {
        let room = submissions as usize;
        for slot in 0..room {
            queue.post_submission_for_test(
                slot,
                queue::Submission::simple(queue::Opcode::Echo, posted, u64::from(posted)),
            );
        }
        queue.publish_tail_for_test(posted + room as u32);
        let served = queue.serve(&shared_handle_table_or_false!(8));
        if served == 0 {
            break;
        }
        posted += served as u32;
    }

    // Nothing is reaped, so every slot is spoken for or holds a completion and nothing is admitted.
    let refuses = queue.reserve_for_test(1) == 0 && queue.is_consistent();

    // A strict-prefix claim must not imply that unread completions were consumed.
    let head_before = queue.completion_head();
    let partial = posted / 2;
    queue.publish_completion_head_for_test(partial);
    let reaped = queue.reap_up_to(queue.completion_head());
    let followed = head_before == 0 && reaped == partial as usize;

    // Room for exactly what was reaped, and no more.
    let resumes = queue.reserve_for_test(partial as usize) == partial as usize
        && queue.reserve_for_test(1) == 0
        && queue.is_consistent();

    // The user head is an untrusted claim; an out-of-range claim reaps nothing.
    let Some(liar) = queue::Queue::new() else {
        return false;
    };
    for slot in 0..2 {
        liar.post_submission_for_test(
            slot,
            queue::Submission::simple(queue::Opcode::Echo, slot as u32, 0),
        );
    }
    liar.publish_tail_for_test(2);
    if liar.serve(&shared_handle_table_or_false!(8)) != 2 {
        return false;
    }
    liar.publish_completion_head_for_test(u32::MAX);
    let far_ahead = liar.reap_up_to(liar.completion_head()) == 0;
    let one = liar.reap_up_to(1) == 1;
    let just_behind = liar.reap_up_to(0) == 0;
    let lied = far_ahead && one && just_behind && liar.is_consistent();

    let quiet_completion_counted =
        queue::Queue::new().is_some_and(|queue| queue.quiet_completion_is_counted_for_test());
    posted > 0 && partial > 0 && refuses && followed && resumes && lied && quiet_completion_counted
}

pub(super) fn a_ring_announces_its_shape_and_keeps_its_pages() -> bool {
    /// A user address nothing else uses.
    const AT: u64 = user::STACK + 0x60_0000;

    let Some(mut queue) = queue::Queue::new() else {
        return false;
    };
    // Compare the exported shape, not `Queue::new`'s constructor invariants.
    let (version, submissions, completions) = queue.shape();

    // Left to prove: the header page was published to match, which is the whole ABI a program
    // reads.
    let announced = queue.published_shape() == (version, submissions, completions)
        && version == ring_proto::VERSION;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        if Arc::get_mut(&mut queue).is_none_or(|queue| queue.install(space, AT).is_err()) {
            return false;
        }

        let reachable = space.lookup(AT).is_some_and(|(_, flags)| {
            flags.contains(Flags::USER) && flags.contains(Flags::WRITABLE)
        });

        // The space cannot give the pages away: `unmap` reads the owned bit before minting a
        // `Segment`, and these frames were never the space's to own.
        let unowned = matches!(space.unmap(AT, PAGE_SIZE), Err(MapError::NotOwned(AT)));

        announced && reachable && unowned
    })
}

pub(super) fn a_word_that_does_not_fit_is_refused_rather_than_truncated() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let held = crate::object::bootstrap_identity();
    let table = shared_handle_table_or_false!(8);
    let full = Rights::UNIVERSAL;
    let Ok(source) = table.insert(held, full) else {
        return false;
    };
    let Some((widened, cancelled)) =
        serve_an_unfit_rights_word_and_token(&queue, &table, source, full)
    else {
        return false;
    };
    // A file offset past every file this kernel can hold.
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

    widened.status == queue::STATUS_DENIED
        // The timer is still in flight: a token that does not fit names no operation.
        && cancelled.status == queue::STATUS_NOT_FOUND
        && queue.in_flight() == 1
        // The reads answer nothing rather than the file's first bytes.
        && at_max.status == queue::STATUS_OK
        && at_max.value == 0
        && past.status == queue::STATUS_OK
        && past.value == 0
        && untouched == [0u8; 32]
}

fn serve_an_unfit_rights_word_and_token(
    queue: &Arc<queue::Queue>,
    table: &Arc<HandleTable>,
    source: u32,
    full: Rights,
) -> Option<(queue::Completion, queue::Completion)> {
    // The low half of this rights word is valid and the high half is not. A cast reads it as the
    // low half; `try_from` refuses it whole.
    queue.post_submission_for_test(
        0,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0xC001,
            source,
            (0xFFFF_FFFFu64 << 32) | u64::from(full.bits()),
        ),
    );

    // An operation to cancel, and a token too wide to be its token.
    queue.post_submission_for_test(
        1,
        queue::Submission::simple(queue::Opcode::Timer, 0xDEAD, 3_600_000_000_000),
    );
    queue.publish_tail_for_test(2);
    if queue.serve(table) != 1 {
        return None;
    }
    let widened = queue.read_completion(0);

    queue.post_submission_for_test(
        2,
        queue::Submission::simple(queue::Opcode::Cancel, 0xC002, (1u64 << 32) | 0xDEAD),
    );
    queue.publish_tail_for_test(3);
    if queue.serve(table) != 1 {
        return None;
    }
    Some((widened, queue.read_completion(1)))
}

fn read_past_every_file(
    queue: &Arc<queue::Queue>,
    table: &Arc<HandleTable>,
    file: u32,
) -> Option<(queue::Completion, queue::Completion)> {
    queue.post_submission_for_test(
        4,
        queue::Submission::about(queue::Opcode::Read, 0xC003, file, 512)
            .offset(u64::from(u32::MAX))
            .length(16),
    );
    queue.post_submission_for_test(
        5,
        queue::Submission::about(queue::Opcode::Read, 0xC004, file, 528)
            .offset(1u64 << 32)
            .length(16),
    );
    queue.publish_tail_for_test(6);
    let before_read = queue.served_count();
    if queue.serve(table) != 0 || !runs_until(5_000, || queue.served_count() == before_read + 2) {
        return None;
    }
    Some((queue.read_completion(3), queue.read_completion(4)))
}

fn open_the_small_file_in(
    queue: &Arc<queue::Queue>,
    table: &alloc::sync::Arc<crate::handle::HandleTable>,
    slot: usize,
) -> Option<u32> {
    let root = vfs::root()?;
    let object = crate::object::directory(root);
    let directory = table
        .insert(object, Rights::UNIVERSAL.union(Rights::TRAVERSE))
        .ok()?;

    if !queue.write_data_for_test(0, SMALL_NAME) {
        return None;
    }
    queue.post_submission_for_test(
        slot,
        queue::Submission::about(queue::Opcode::OpenAt, 0x0F11, directory, 0)
            .length(SMALL_NAME.len() as u32),
    );
    queue.publish_tail_for_test(slot as u32 + 1);
    let before = queue.served_count();
    if queue.serve(table) != 0 || !runs_until(5_000, || queue.served_count() == before + 1) {
        return None;
    }
    let completion = queue.read_completion(2);
    (completion.status == queue::STATUS_OK).then_some(completion.value as u32)
}

pub(super) fn a_completion_carries_a_file_handle() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(root) = vfs::root() else {
        return false;
    };
    let object = crate::object::directory(root);

    let table = shared_handle_table_or_false!(8);
    let held = Rights::UNIVERSAL
        .union(Rights::TRAVERSE)
        .union(Rights::ENUMERATE);
    let Ok(directory) = table.insert(object, held) else {
        return false;
    };

    if !queue.write_data_for_test(0, SMALL_NAME) {
        return false;
    }
    queue.post_submission_for_test(
        0,
        queue::Submission::about(queue::Opcode::OpenAt, 0x09E4, directory, 0)
            .length(SMALL_NAME.len() as u32),
    );
    queue.publish_tail_for_test(1);

    let before = queue.served_count();
    if queue.serve(&table) != 0 || !runs_until(5_000, || queue.served_count() == before + 1) {
        return false;
    }
    let completion = queue.read_completion(0);
    if completion.status != queue::STATUS_OK {
        return false;
    }

    let file = completion.value as u32;
    let Ok((object, granted)) = table.resolve(file) else {
        return false;
    };
    if object.as_file().is_none() {
        return false;
    }

    // Read and never write: there is no write path, and a right that grants nothing is a syscall
    // name in disguise.
    granted == Rights::UNIVERSAL.union(Rights::READ)
        && !granted.contains(Rights::WRITE)
        && !granted.contains(Rights::TRAVERSE)
        && table.live() == 2
        && table.promised() == 0
}

pub(super) fn opening_a_node_does_not_restore_universal_rights() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(root) = vfs::root() else {
        return false;
    };
    let object = crate::object::directory(root);

    let table = shared_handle_table_or_false!(8);
    let Ok(directory) = table.insert(object, Rights::TRAVERSE) else {
        return false;
    };
    const DIRECTORY_AT: u64 = 64;
    if !queue.write_data_for_test(0, SMALL_NAME)
        || !queue.write_data_for_test(DIRECTORY_AT as usize, DIRECTORY_NAME)
    {
        return false;
    }
    for (slot, token, at, name) in [
        (0, 0xA771, 0, SMALL_NAME),
        (1, 0xA772, DIRECTORY_AT, DIRECTORY_NAME),
    ] {
        queue.post_submission_for_test(
            slot,
            queue::Submission::about(queue::Opcode::OpenAt, token, directory, at)
                .length(name.len() as u32),
        );
    }
    queue.publish_tail_for_test(2);
    let before = queue.served_count();
    if queue.serve(&table) != 0 || !runs_until(5_000, || queue.served_count() == before + 2) {
        return false;
    }

    let file = queue.read_completion(0);
    let subdirectory = queue.read_completion(1);
    if file.status != queue::STATUS_OK || subdirectory.status != queue::STATUS_OK {
        return false;
    }
    let Ok((file, file_rights)) = table.resolve(file.value as u32) else {
        return false;
    };
    let Ok((subdirectory, directory_rights)) = table.resolve(subdirectory.value as u32) else {
        return false;
    };

    file.as_file().is_some()
        && file_rights == Rights::READ
        && subdirectory.as_directory().is_some()
        && directory_rights == Rights::TRAVERSE
        && table.live() == 3
        && table.promised() == 0
}

pub(super) fn a_read_is_a_submission_with_its_own_offset() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(file) = open_the_small_file(&queue) else {
        return false;
    };
    let (table, handle) = file;

    for (slot, token, offset, at, length) in [
        (1usize, 0x00A1u32, 0u64, 64u64, 16u32),
        (2, 0x00A2, 8, 128, 16),
    ] {
        queue.post_submission_for_test(
            slot,
            queue::Submission::about(queue::Opcode::Read, token, handle, at)
                .offset(offset)
                .length(length),
        );
    }
    queue.publish_tail_for_test(3);
    let before = queue.served_count();
    if queue.serve(&table) != 0 || !runs_until(5_000, || queue.served_count() == before + 2) {
        return false;
    }

    let first = queue.read_completion(1);
    let second = queue.read_completion(2);
    if first.status != queue::STATUS_OK || second.status != queue::STATUS_OK {
        return false;
    }
    if first.value != 16 || second.value != 16 {
        return false;
    }

    let mut head = [0u8; 16];
    let mut later = [0u8; 16];
    if !queue.read_data_for_test(64, &mut head) || !queue.read_data_for_test(128, &mut later) {
        return false;
    }

    // The bytes are the file's at the offsets asked for, and the two differ, which a read that
    // ignored the offset could not manage.
    head == SMALL_CONTENT[..16] && later == SMALL_CONTENT[8..24] && head != later
}

fn small_file_authority() -> Option<(Arc<HandleTable>, u32)> {
    let root = vfs::root()?;
    let vfs::Node::File(file) = vfs::resolve(root, SMALL_NAME).ok()? else {
        return None;
    };
    let table = Arc::new(HandleTable::new(16).ok()?);
    let handle = table
        .insert(
            crate::object::file(file),
            Rights::UNIVERSAL.union(Rights::READ),
        )
        .ok()?;
    Some((table, handle))
}

pub(super) fn a_cancelled_open_releases_its_reserved_handle_before_quiet() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };
    let table = shared_handle_table_or_false!(4);
    let Ok(directory) = table.insert(
        crate::object::directory(root),
        Rights::UNIVERSAL.union(Rights::TRAVERSE),
    ) else {
        return false;
    };
    let Some(ring) = queue::Queue::new() else {
        return false;
    };
    let Some(retry) = queue::Queue::new() else {
        return false;
    };
    if !ring.write_data_for_test(0, SMALL_NAME) || !retry.write_data_for_test(0, SMALL_NAME) {
        return false;
    }

    queue::pause_worker_for_test(true);
    if !runs_until(2_000, queue::worker_is_paused_for_test) {
        queue::pause_worker_for_test(false);
        return false;
    }
    let queued = queue::queued_work_for_test();
    let cancelled = a_queued_open_cancels_without_its_handle(&ring, &table, directory, queued);
    let refused = a_refused_cancellation_keeps_the_open_queued(&retry, &table, directory, queued);

    let served = retry.served_count();
    queue::pause_worker_for_test(false);
    let worker_finished = runs_until(5_000, || retry.served_count() == served + 1);
    let completion = retry.read_completion(1);
    let installed = completion.token == 0x0A81
        && completion.status == queue::STATUS_OK
        && table
            .resolve(completion.value as u32)
            .is_ok_and(|(object, rights)| {
                object.as_file().is_some() && rights == Rights::UNIVERSAL.union(Rights::READ)
            });

    cancelled
        && refused
        && worker_finished
        && installed
        && retry.is_quiet()
        && retry.in_flight() == 0
        && table.live() == 2
        && table.promised() == 0
        && queue::queued_work_for_test() == queued
}

fn a_queued_open_cancels_without_its_handle(
    ring: &Arc<queue::Queue>,
    table: &Arc<HandleTable>,
    directory: u32,
    queued: usize,
) -> bool {
    ring.post_submission_for_test(
        0,
        queue::Submission::about(queue::Opcode::OpenAt, 0x0A71, directory, 0)
            .length(SMALL_NAME.len() as u32),
    );
    ring.publish_tail_for_test(1);
    let admitted = ring.serve(table) == 0
        && ring.in_flight() == 1
        && table.live() == 1
        && table.promised() == 1
        && queue::queued_work_for_test() == queued + 1;

    ring.post_submission_for_test(
        1,
        queue::Submission::simple(queue::Opcode::Cancel, 0x0A72, 0x0A71),
    );
    ring.publish_tail_for_test(2);
    let cancelled = ring.serve(table) == 1;
    let target = ring.read_completion(0);
    let canceller = ring.read_completion(1);

    admitted
        && cancelled
        && target.token == 0x0A71
        && target.status == queue::STATUS_CANCELLED
        && canceller.token == 0x0A72
        && canceller.status == queue::STATUS_OK
        && ring.is_quiet()
        && ring.in_flight() == 0
        && table.live() == 1
        && table.promised() == 0
        && queue::queued_work_for_test() == queued
}

// A refused cancellation leaves the open admitted, so the worker still installs its handle.
fn a_refused_cancellation_keeps_the_open_queued(
    retry: &Arc<queue::Queue>,
    table: &Arc<HandleTable>,
    directory: u32,
    queued: usize,
) -> bool {
    retry.post_submission_for_test(
        0,
        queue::Submission::about(queue::Opcode::OpenAt, 0x0A81, directory, 0)
            .length(SMALL_NAME.len() as u32),
    );
    retry.publish_tail_for_test(1);
    let admitted_for_retry = retry.serve(table) == 0
        && retry.in_flight() == 1
        && table.live() == 1
        && table.promised() == 1
        && queue::queued_work_for_test() == queued + 1;
    retry.refuse_next_post_for_test();
    retry.post_submission_for_test(
        1,
        queue::Submission::simple(queue::Opcode::Cancel, 0x0A82, 0x0A81),
    );
    retry.publish_tail_for_test(2);
    let cancellation_refused = retry.serve(table) == 1
        && retry.read_completion(0).token == 0x0A82
        && retry.read_completion(0).status == queue::STATUS_NO_ROOM
        && retry.in_flight() == 1
        && table.live() == 1
        && table.promised() == 1
        && queue::queued_work_for_test() == queued + 1;

    admitted_for_retry && cancellation_refused
}

pub(super) fn file_work_pressure_is_bounded_at_admission() -> bool {
    let Some((table, handle)) = small_file_authority() else {
        return false;
    };
    let Some(work) = queue::Queue::new() else {
        return false;
    };
    let Some(overflow) = queue::Queue::new() else {
        return false;
    };

    queue::pause_worker_for_test(true);
    let runs_before = queue::worker_runs_for_test();
    for slot in 0..8 {
        work.post_submission_for_test(
            slot,
            queue::Submission::about(queue::Opcode::Read, 0x8100 + slot as u32, handle, 0)
                .offset(slot as u64)
                .length(16),
        );
    }
    work.publish_tail_for_test(8);
    let eight_wait =
        work.serve(&table) == 0 && work.in_flight() == 8 && queue::queued_work_for_test() == 8;

    overflow.post_submission_for_test(
        0,
        queue::Submission::about(queue::Opcode::Read, 0x8199, handle, 0).length(16),
    );
    overflow.publish_tail_for_test(1);
    let ninth_refused = overflow.serve(&table) == 1
        && overflow.read_completion(0).status == queue::STATUS_NO_ROOM
        && overflow.in_flight() == 0;

    queue::pause_worker_for_test(false);
    let drained = runs_until(10_000, || {
        work.is_quiet() && queue::queued_work_for_test() == 0
    });
    let mut exact = true;
    for slot in 0..8 {
        let completion = work.read_completion(slot);
        exact &= completion.token == 0x8100 + slot as u32
            && completion.status == queue::STATUS_OK
            && completion.value == 16;
    }

    eight_wait
        && ninth_refused
        && drained
        && exact
        && queue::worker_runs_for_test() == runs_before + 8
        && work.outstanding() == 0
        && work.is_consistent()
}

pub(super) fn a_delayed_read_preserves_interrupt_admission_and_exact_teardown() -> bool {
    if !the_clock_is_running() {
        return false;
    }
    let refused_input_releases_hold = refused_delayed_input_releases_the_hold();
    let Some((table, handle)) = small_file_authority() else {
        return false;
    };
    let Some(ring) = queue::Queue::new() else {
        return false;
    };
    ring.post_submission_for_test(
        0,
        queue::Submission::about(queue::Opcode::Read, 0xD200, handle, 64).length(16),
    );
    ring.publish_tail_for_test(1);

    nvme::hold_command_for_test(true);
    let served_before = ring.served_count();
    let deferred = ring.serve(&table) == 0 && ring.in_flight() == 1;
    let reached_device = runs_until(2_000, nvme::command_is_held_for_test);
    let retained = ring.abandon() == 0 && !ring.is_quiet() && Arc::strong_count(&ring) > 1;

    let interrupt_admission = interrupt::are_enabled();
    let key = input::arm_key(ps2_keys::Key::C);
    // Kept in step with STORAGE_DELAY_MARKER in `xtask/src/qemu.rs`.
    awaiting(
        "selftest: awaiting storage-delay keystroke",
        "  PRESS  c   to continue the storage round",
    );
    // This runs before the permanent fallback starts so later scheduler checks keep their tickless
    // baseline. Draining here while the storage worker is held proves polling and unrelated IRQ
    // admission can progress together.
    let arrived = finish_delayed_read_input(
        || {
            runs_until(input::HUMAN_WAIT_MS, || {
                input::poll();
                key.is_some_and(input::armed_key_arrived)
            })
        },
        input::try_read_key_for_test,
    );
    let input_progress = arrived && interrupt_admission && interrupt::are_enabled();

    let completed = runs_until(5_000, || ring.is_quiet());
    let completion = ring.read_completion(0);
    let mut bytes = [0u8; 16];
    let copied = ring.read_data_for_test(64, &mut bytes);
    let exact = ring.served_count() == served_before + 1
        && completion.token == 0xD200
        && completion.status == queue::STATUS_OK
        && completion.value == 16
        && copied
        && bytes == SMALL_CONTENT[..16]
        && ring.outstanding() == 0
        && ring.abandon() == 0;
    info!(
        "selftest: delayed read evidence deferred={deferred} device={reached_device} retained={retained} polled={input_progress} completed={completed} exact={exact}"
    );

    refused_input_releases_hold
        && deferred
        && reached_device
        && retained
        && input_progress
        && completed
        && exact
}

fn finish_delayed_read_input(
    wait: impl FnOnce() -> bool,
    mut read: impl FnMut() -> Option<input::Keystroke>,
) -> bool {
    let progressed = wait();
    let exact_pair = progressed
        && read().is_some_and(|pressed| {
            pressed.event.key == ps2_keys::Key::C
                && pressed.event.state == ps2_keys::KeyState::Pressed
        })
        && read().is_some_and(|released| {
            released.event.key == ps2_keys::Key::C
                && released.event.state == ps2_keys::KeyState::Released
        });
    nvme::hold_command_for_test(false);
    exact_pair
}

fn refused_delayed_input_releases_the_hold() -> bool {
    let pair = [
        input::Keystroke {
            event: ps2_keys::KeyEvent {
                key: ps2_keys::Key::C,
                state: ps2_keys::KeyState::Pressed,
            },
            character: Some('c'),
        },
        input::Keystroke {
            event: ps2_keys::KeyEvent {
                key: ps2_keys::Key::C,
                state: ps2_keys::KeyState::Released,
            },
            character: None,
        },
    ];
    let clock_running = the_clock_is_running();
    let clockless = time::without_calibration_for_test(|| {
        refused_delayed_input_round(|| runs_until(0, || true), &pair, 0)
    });
    let expired = refused_delayed_input_round(|| runs_until(0, || false), &pair, 0);
    let missing_press = refused_delayed_input_round(|| true, &[], 1);
    let missing_release = refused_delayed_input_round(|| true, &pair[..1], 2);

    clock_running && clockless && expired && missing_press && missing_release
}

fn refused_delayed_input_round(
    wait: impl FnOnce() -> bool,
    events: &[input::Keystroke],
    expected_reads: usize,
) -> bool {
    let mut events = events.iter().copied();
    let mut reads = 0;
    nvme::hold_command_for_test(true);
    let armed = nvme::hold_requested_for_test();
    let accepted = finish_delayed_read_input(wait, || {
        reads += 1;
        events.next()
    });
    let released = !nvme::hold_requested_for_test();
    // A missing-release mutation must fail this check without holding later I/O.
    nvme::hold_command_for_test(false);

    armed && !accepted && released && reads == expected_reads
}

pub(super) fn a_ring_operation_cannot_reach_past_its_rights_or_its_pages() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(root) = vfs::root() else {
        return false;
    };
    let table = shared_handle_table_or_false!(8);
    let Some((unwalkable, walkable, unreadable, readable)) = reach_fixture(&queue, &table, root)
    else {
        return false;
    };

    let before = nvme::command_count_for_test();
    let Some(refused) = opens_are_refused(&queue, &table, unwalkable, walkable) else {
        return false;
    };

    // Device-access counts distinguish preflight refusal from a late error after I/O.
    queue.post_submission_for_test(
        4,
        queue::Submission::about(queue::Opcode::Read, 0xD5, unreadable, 0).length(16),
    );
    queue.post_submission_for_test(
        5,
        queue::Submission::about(queue::Opcode::Read, 0xD6, unreadable, 4090).length(16),
    );
    // This one reaches the bound: the device must still not be asked, or the bound is checked with
    // the work already done.
    queue.post_submission_for_test(
        6,
        queue::Submission::about(queue::Opcode::Read, 0xD7, readable, 4090).length(16),
    );
    queue.publish_tail_for_test(7);
    let served = queue.serve(&table) == 3;
    let after = nvme::command_count_for_test();

    let denied = queue.read_completion(4).status == queue::STATUS_DENIED
        && queue.read_completion(5).status == queue::STATUS_DENIED
        && queue.read_completion(6).status == queue::STATUS_BAD_BUFFER;
    let untouched = before == after;

    // Nothing was promised and kept: every refusal above gave its slot back.
    refused && served && denied && untouched && table.live() == 4 && table.promised() == 0
}

fn reach_fixture(
    queue: &Arc<queue::Queue>,
    table: &Arc<HandleTable>,
    root: vfs::Directory,
) -> Option<(u32, u32, u32, u32)> {
    // A directory held without TRAVERSE: it can be listed and not walked.
    let object = crate::object::directory(root);
    let unwalkable = table
        .insert(object, Rights::UNIVERSAL.union(Rights::ENUMERATE))
        .ok()?;
    // A walkable one, for the paths that are about the path.
    let object = crate::object::directory(root);
    let walkable = table
        .insert(object, Rights::UNIVERSAL.union(Rights::TRAVERSE))
        .ok()?;

    if !queue.write_data_for_test(0, SMALL_NAME) || !queue.write_data_for_test(256, b"..") {
        return None;
    }

    // A file held without READ, so its read is refused for the right rather than the file.
    let vfs::Node::File(file) = vfs::resolve(root, SMALL_NAME).ok()? else {
        return None;
    };
    let object = crate::object::file(file);
    let unreadable = table.insert(object, Rights::UNIVERSAL).ok()?;
    // Grant `READ` so this case reaches the buffer bound.
    let object = crate::object::file(file);
    let readable = table
        .insert(object, Rights::UNIVERSAL.union(Rights::READ))
        .ok()?;
    Some((unwalkable, walkable, unreadable, readable))
}

fn opens_are_refused(
    queue: &Arc<queue::Queue>,
    table: &Arc<HandleTable>,
    unwalkable: u32,
    walkable: u32,
) -> Option<bool> {
    let asked = [
        // No TRAVERSE.
        (
            0usize,
            0xD1u32,
            unwalkable,
            0u64,
            SMALL_NAME.len() as u32,
            queue::STATUS_DENIED,
        ),
        // A path that would leave the directory.
        (1, 0xD2, walkable, 256, 2, queue::STATUS_BAD_PATH),
        // A path buffer outside the data area.
        (2, 0xD3, walkable, 8192, 8, queue::STATUS_BAD_BUFFER),
        // A path of no length at all.
        (3, 0xD4, walkable, 0, 0, queue::STATUS_BAD_BUFFER),
    ];
    for (slot, token, handle, at, length, _) in asked {
        queue.post_submission_for_test(
            slot,
            queue::Submission::about(queue::Opcode::OpenAt, token, handle, at).length(length),
        );
    }
    queue.publish_tail_for_test(asked.len() as u32);
    let served_before = queue.served_count();
    if queue.serve(table) != asked.len() - 1
        || !runs_until(5_000, || {
            queue.served_count() == served_before + asked.len()
        })
    {
        return None;
    }

    Some(asked.iter().all(|(_, token, _, _, _, status)| {
        (0..asked.len()).any(|slot| {
            let completion = queue.read_completion(slot);
            completion.token == *token && completion.status == *status
        })
    }))
}

fn open_the_small_file(
    queue: &Arc<queue::Queue>,
) -> Option<(alloc::sync::Arc<crate::handle::HandleTable>, u32)> {
    let root = vfs::root()?;
    let object = crate::object::directory(root);
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8).ok()?);
    let held = Rights::UNIVERSAL.union(Rights::TRAVERSE);
    let directory = table.insert(object, held).ok()?;

    if !queue.write_data_for_test(0, SMALL_NAME) {
        return None;
    }
    queue.post_submission_for_test(
        0,
        queue::Submission::about(queue::Opcode::OpenAt, 1, directory, 0)
            .length(SMALL_NAME.len() as u32),
    );
    queue.publish_tail_for_test(1);
    let before = queue.served_count();
    if queue.serve(&table) != 0 || !runs_until(5_000, || queue.served_count() == before + 1) {
        return None;
    }
    let completion = queue.read_completion(0);
    (completion.status == queue::STATUS_OK).then_some((table, completion.value as u32))
}

pub(super) fn work_in_flight_is_ended_before_the_ring_is_released() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = shared_handle_table_or_false!(8);

    // Two operations, an hour out, so nothing but the teardown can end them.
    for (slot, token) in [(0usize, 0xA1u32), (1, 0xA2)] {
        queue.post_submission_for_test(
            slot,
            queue::Submission::simple(queue::Opcode::Timer, token, 3_600_000_000_000),
        );
    }
    queue.publish_tail_for_test(2);

    let in_flight = queue.serve(&table) == 0 && queue.in_flight() == 2 && queue.outstanding() == 2;

    let ended = queue.abandon();

    // Each got the completion it was owed, and it says what happened rather than pretending to be
    // a result.
    let first = queue.read_completion(0);
    let second = queue.read_completion(1);
    let completed = first.token == 0xA1
        && first.status == queue::STATUS_CANCELLED
        && second.token == 0xA2
        && second.status == queue::STATUS_CANCELLED;

    in_flight
        && ended == 2
        && completed
        // Nothing left could touch the pages, and `outstanding` would otherwise say work is in
        // flight for ever.
        && queue.is_quiet()
        && queue.in_flight() == 0
        && queue.outstanding() == 0
        && queue.is_consistent()
        // A second sweep finds nothing: teardown is idempotent, because `reclaim` is not the only
        // path to a zombie.
        && queue.abandon() == 0
}

pub(super) fn a_refused_settlement_retains_every_owner() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(cancelled) = queue::Queue::new() else {
        return false;
    };
    let cancel_table = shared_handle_table_or_false!(8);
    let Some(cancellation) = a_refused_cancellation_settles_on_abandon(&cancelled, &cancel_table)
    else {
        return false;
    };
    let Some(expired) = queue::Queue::new() else {
        return false;
    };
    let expire_table = shared_handle_table_or_false!(8);
    let Some(expiry) = a_refused_expiry_settles_on_the_next_sweep(&expired, &expire_table) else {
        return false;
    };

    while user::collect().is_some() {}
    let refusals_before = user::refused_ring_reclaims();
    let retirements_before = thread::retirements();
    let Ok(spawned) = user::run_faulting() else {
        return false;
    };
    let Some((ring, table)) = user::authority_for_test(spawned.slot) else {
        return false;
    };
    ring.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xDEAD, u64::MAX),
    );
    ring.publish_tail_for_test(1);
    if ring.serve(&table) != 0 || ring.in_flight() != 1 {
        return false;
    }
    ring.refuse_next_post_for_test();
    queue::pause_worker_for_test(true);
    let retired =
        runs_until(2000, || thread::retirements() > retirements_before) && user::has_zombies();
    let first_refused = retired
        && matches!(user::collect(), user::CollectOutcome::Retained)
        && user::has_zombies()
        && user::refused_ring_reclaims() == refusals_before + 1
        && ring.in_flight() == 1
        && ring.outstanding() == 1
        && ring.settlement_refusals_for_test() == 1;
    let collected = user::collect().is_some_and(|reclaimed| reclaimed.abandoned == 1)
        && !user::has_zombies()
        && ring.read_completion(0).token == 0xDEAD
        && ring.read_completion(0).status == queue::STATUS_CANCELLED
        && ring.is_quiet()
        && ring.outstanding() == 0;
    queue::pause_worker_for_test(false);

    cancellation && expiry && first_refused && collected
}

fn a_refused_cancellation_settles_on_abandon(
    cancelled: &Arc<queue::Queue>,
    cancel_table: &Arc<HandleTable>,
) -> Option<bool> {
    cancelled.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xCA11, u64::MAX),
    );
    cancelled.publish_tail_for_test(1);
    if cancelled.serve(cancel_table) != 0 || cancelled.in_flight() != 1 {
        return None;
    }
    cancelled.post_submission_for_test(
        1,
        queue::Submission::simple(queue::Opcode::Cancel, 0xCA12, 0xCA11),
    );
    cancelled.publish_tail_for_test(2);
    cancelled.refuse_next_post_for_test();
    let cancel_refused = cancelled.serve(cancel_table) == 1
        && cancelled.read_completion(0).token == 0xCA12
        && cancelled.read_completion(0).status == queue::STATUS_NO_ROOM
        && cancelled.in_flight() == 1
        && cancelled.outstanding() == 1
        && cancelled.settlement_refusals_for_test() == 1;
    let cancel_retried = cancelled.abandon() == 1
        && cancelled.read_completion(1).token == 0xCA11
        && cancelled.read_completion(1).status == queue::STATUS_CANCELLED
        && cancelled.is_quiet()
        && cancelled.outstanding() == 0;
    Some(cancel_refused && cancel_retried)
}

fn a_refused_expiry_settles_on_the_next_sweep(
    expired: &Arc<queue::Queue>,
    expire_table: &Arc<HandleTable>,
) -> Option<bool> {
    expired.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xE771, u64::MAX),
    );
    expired.publish_tail_for_test(1);
    if expired.serve(expire_table) != 0 || expired.in_flight() != 1 {
        return None;
    }
    expired.refuse_next_post_for_test();
    let expiry_refused = expired.expire(u64::MAX) == 0
        && expired.in_flight() == 1
        && expired.outstanding() == 1
        && expired.settlement_refusals_for_test() == 1;
    let expiry_retried = expired.expire(u64::MAX) == 1
        && expired.expire(u64::MAX) == 0
        && expired.read_completion(0).token == 0xE771
        && expired.read_completion(0).status == queue::STATUS_OK
        && expired.is_quiet()
        && expired.outstanding() == 0;
    Some(expiry_refused && expiry_retried)
}

pub(super) fn a_promise_outlives_the_program_that_made_it() -> bool {
    let held = crate::object::bootstrap_identity();
    let second = crate::object::bootstrap_identity();

    let table = shared_handle_table_or_false!(4);
    let foreign = shared_handle_table_or_false!(2);
    let watching = alloc::sync::Arc::downgrade(&table);

    let full = Rights::UNIVERSAL;
    if table.insert(held, full).is_err() {
        return false;
    }
    let Ok(reservation) = table.reserve_slot() else {
        return false;
    };
    let promised = table.live() == 1 && table.promised() == 1;

    // The program dies, and the only reference left to its table is the one inside the promise.
    drop(table);

    let Some(orphan) = watching.upgrade() else {
        // A table freed under a live promise is a use-after-free awaiting its completion, not
        // merely a failed check.
        return false;
    };

    // Surviving is not working: the promise is spent against the orphaned table, as a completion
    // after the program's death would.
    let usage_before = HandleTable::arena_usage_for_test();
    let Ok(handle) = reservation.install(second, full, full) else {
        return false;
    };
    let Some(slot) = orphan.slot_of(handle) else {
        return false;
    };
    let foreign_handle = foreign.handle_for(slot);
    let usage_after = HandleTable::arena_usage_for_test();
    let installed = orphan.resolve(handle).is_ok()
        && matches!(foreign.resolve(foreign_handle), Err(HandleError::Stale));
    let accounted = orphan.live() == 2
        && orphan.promised() == 0
        && foreign.live() == 0
        && foreign.promised() == 0
        && usage_after.0 == usage_before.0
        && usage_after.1 == usage_before.1 + 1;

    // With the promise spent, the table goes with its last reference; a `Weak` that still upgrades
    // here is a table nothing will ever free.
    drop(orphan);
    drop(foreign);
    let released = watching.upgrade().is_none();

    promised && installed && accounted && released
}

pub(super) fn an_operation_completes_after_its_deadline_and_only_once() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = shared_handle_table_or_false!(8);

    const DELAY_NS: u64 = 2_000_000;
    let submitted_at = crate::time::now_ns();
    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0x7117, DELAY_NS),
    );
    queue.publish_tail_for_test(1);

    let deferred = queue.serve(&table) == 0 && queue.in_flight() == 1;
    let Some(deadline) = queue.earliest_deadline() else {
        return false;
    };

    // A million reads: the deadline is two milliseconds out on a running clock, so a deadline
    // this does not reach is wrong rather than slow.
    if !spins_until(1_000_000, || crate::time::now_ns() >= deadline) {
        return false;
    }

    // At the instant before the deadline nothing is due, even once the clock has passed it.
    let early = queue.expire(deadline - 1) == 0 && queue.in_flight() == 1;
    let due = queue.expire(crate::time::now_ns());
    let completion = queue.read_completion(0);

    deferred
        && early
        && due == 1
        // Only once: a second sweep finds nothing, the one-completion rule seen where a deferral
        // could break it.
        && queue.expire(crate::time::now_ns()) == 0
        && queue.in_flight() == 0
        && completion.token == 0x7117
        && completion.status == queue::STATUS_OK
        // Completed *after* the deadline, not merely after the submission.
        && completion.value >= submitted_at.saturating_add(DELAY_NS)
}

pub(super) fn a_cancelled_operation_and_its_cancellation_complete_separately() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = shared_handle_table_or_false!(8);

    // An hour away, so nothing but the cancellation can end it.
    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xDEAD, 3_600_000_000_000),
    );
    queue.publish_tail_for_test(1);
    if queue.serve(&table) != 0 || queue.in_flight() != 1 {
        return false;
    }

    queue.post_submission_for_test(
        1,
        queue::Submission::simple(queue::Opcode::Cancel, 0xC0DE, 0xDEAD),
    );
    queue.publish_tail_for_test(2);
    if queue.serve(&table) != 1 {
        return false;
    }

    // The target's completion, posted from inside the cancellation, then the cancellation's own.
    let target = queue.read_completion(0);
    let canceller = queue.read_completion(1);

    let separate = target.token == 0xDEAD
        && target.status == queue::STATUS_CANCELLED
        && canceller.token == 0xC0DE
        && canceller.status == queue::STATUS_OK
        && canceller.value == u64::from(0xDEADu32);

    // A second cancellation finds nothing: advisory, as io_uring's `-EALREADY`, for a race the
    // program cannot avoid.
    queue.post_submission_for_test(
        2,
        queue::Submission::simple(queue::Opcode::Cancel, 0xC0DF, 0xDEAD),
    );
    queue.publish_tail_for_test(3);
    let again =
        queue.serve(&table) == 1 && queue.read_completion(2).status == queue::STATUS_NOT_FOUND;

    separate
        && again
        && queue.in_flight() == 0
        && queue.is_consistent()
        && queue::pending_identity_exhaustion_is_terminal_for_test()
}

pub(super) fn every_submission_gets_exactly_one_completion() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };

    let asked = [
        queue::Submission::simple(queue::Opcode::Echo, 0x1111, 0xDEAD_BEEF),
        queue::Submission::simple(queue::Opcode::Uptime, 0x2222, 0),
        queue::Submission::simple(queue::Opcode::Unknown(0xFFFF), 0x3333, 0),
    ];

    for (slot, submission) in asked.iter().enumerate() {
        queue.post_submission_for_test(slot, *submission);
    }
    queue.publish_tail_for_test(asked.len() as u32);

    let served = queue.serve(&shared_handle_table_or_false!(8));
    // Exactly one each, and no more: a second call finds nothing left.
    let once = served == asked.len() && queue.serve(&shared_handle_table_or_false!(8)) == 0;
    let accounted = queue.outstanding() == 0 && queue.is_consistent();

    let echo = queue.read_completion(0);
    let uptime = queue.read_completion(1);
    let refused = queue.read_completion(2);

    let matched = echo.token == 0x1111
        && echo.status == queue::STATUS_OK
        && echo.value == 0xDEAD_BEEF
        && uptime.token == 0x2222
        && uptime.status == queue::STATUS_OK
        // The value, not only the token: a zeroed `now_ns()` once left this green.
        && uptime.value > 0
        && uptime.value <= crate::time::now_ns()
        && refused.token == 0x3333
        // Refused with a completion: an operation the kernel will not serve still reports, or a
        // caller waits for ever.
        && refused.status == queue::STATUS_UNKNOWN_OPCODE;

    once && accounted && matched
}

pub(super) fn admission_refuses_before_the_completion_queue_can_overflow() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let (_, submissions, completions) = queue.shape();

    let wanted = (completions as usize) * 4;
    let admitted = queue.reserve_for_test(wanted);

    // Bounded by the submission queue, the tighter of the two here, and never at zero.
    let bounded = admitted > 0 && admitted <= submissions as usize;
    let refused = admitted < wanted;
    let consistent = queue.is_consistent();

    // A full ring admits nothing more until something reports.
    let full_refuses = queue.reserve_for_test(1) == 0;

    bounded && refused && consistent && full_refuses
}

pub(super) fn a_rewritten_submission_cannot_change_what_was_served() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };

    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Echo, 0xABCD, 0x0F0F_0F0F),
    );
    queue.publish_tail_for_test(1);
    if queue.serve(&shared_handle_table_or_false!(8)) != 1 {
        return false;
    }
    let posted = queue.read_completion(0);

    // Ring 3 rewrites its submitted entry, as a hostile program would the instant the head moved.
    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Unknown(0xDEAD), 0xBAAD, 0xFFFF_FFFF),
    );

    // The visible rewrite plus original completion proves service used the copied submission.
    let rewritten = queue.read_submission(0);
    let took_effect = rewritten.token == 0xBAAD && rewritten.opcode != queue::Opcode::Echo;
    let unchanged = took_effect
        && posted.token == 0xABCD
        && posted.status == queue::STATUS_OK
        && posted.value == 0x0F0F_0F0F
        // Serving again does not re-serve the rewritten entry: the slot was consumed, and the new
        // bytes are a submission nobody made.
        && queue.serve(&shared_handle_table_or_false!(8)) == 0;

    // Reservations make this runaway tail reach the explicit posted bound instead of an
    // empty-queue early return.
    let Some(runaway) = queue::Queue::new() else {
        return false;
    };
    // Three written under a tail claiming four billion: without the bound the kernel admits and
    // serves all three from slots it was never told about.
    for slot in 0..3 {
        runaway.post_submission_for_test(
            slot,
            queue::Submission::simple(queue::Opcode::Echo, 0x9999, 0),
        );
    }
    runaway.publish_tail_for_test(u32::MAX);
    let runaway_bounded =
        runaway.serve(&shared_handle_table_or_false!(8)) == 0 && runaway.outstanding() == 0;

    queue.publish_tail_for_test(u32::MAX);
    let drained_bounded = queue.serve(&shared_handle_table_or_false!(8)) == 0;

    let Some(partial) = queue::Queue::new() else {
        return false;
    };
    for slot in 0..3 {
        partial.post_submission_for_test(
            slot,
            queue::Submission::simple(queue::Opcode::Echo, 0x5150 + slot as u32, 7),
        );
    }
    partial.publish_tail_for_test(1);
    let only_published = partial.serve(&shared_handle_table_or_false!(8)) == 1;
    let one_reported = partial.read_completion(0).token == 0x5150;
    // Nothing is held open: the one served reported and the two unpublished entries were never
    // admitted, so no reservation outlives the round.
    let still_reserved = partial.outstanding() == 0 && partial.is_consistent();

    unchanged
        && runaway_bounded
        && drained_bounded
        && only_published
        && one_reported
        && still_reserved
}
