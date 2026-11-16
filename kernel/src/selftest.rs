use core::hint::spin_loop;

use boot_record::{Record, Verdict};

use crate::arch::current::debug_exit;
use crate::thread::ThreadContext;
use crate::{error, info, time};

mod address_spaces;
mod clocks;
mod handles;
mod memory;
mod processes;
mod rings;
mod storage;
mod threads;

type Check = (&'static str, fn(&mut ThreadContext) -> bool);

// The unserved-call check runs before any program; the nvme timeout resets storage, so last.
const CHECKS: &[Check] = &[
    ("allocation returns a zeroed frame", |_| {
        memory::allocation_is_zeroed()
    }),
    ("the kernel image obeys write-xor-execute", |_| {
        address_spaces::the_image_obeys_write_xor_execute()
    }),
    (
        "an unserved call is refused without stopping the machine",
        processes::an_unserved_call_is_refused_without_stopping_the_machine,
    ),
    (
        "a faulting program dies alone",
        processes::a_faulting_program_dies_alone,
    ),
    (
        "a direct clock read retires its program",
        processes::a_direct_clock_read_retires_its_program,
    ),
    (
        "a noncanonical user return stack retires only its program",
        processes::a_noncanonical_user_return_stack_retires_only_its_program,
    ),
    (
        "a program cannot aim at the copy window",
        processes::a_program_cannot_aim_at_the_copy_window,
    ),
    (
        "a program cannot read the kernel",
        processes::a_program_cannot_read_the_kernel,
    ),
    (
        "a syscall instruction never enters the kernel",
        processes::a_syscall_instruction_never_enters_the_kernel,
    ),
    (
        "ring wait sleeps only while work is in flight",
        processes::ring_wait_sleeps_only_while_work_is_in_flight,
    ),
    (
        "a thread base is taken only inside user memory",
        processes::a_thread_base_is_taken_only_inside_user_memory,
    ),
    ("two spaces map one address to different memory", |_| {
        address_spaces::two_spaces_map_one_address_to_different_memory()
    }),
    ("every enabled state component survives a switch", |_| {
        threads::every_enabled_state_component_survives_a_switch()
    }),
    (
        "a word that does not fit is refused rather than truncated",
        rings::a_word_that_does_not_fit_is_refused_rather_than_truncated,
    ),
    (
        "opening a node does not restore universal rights",
        rings::opening_a_node_does_not_restore_universal_rights,
    ),
    (
        "a ring operation cannot reach past its rights or its pages",
        rings::a_ring_operation_cannot_reach_past_its_rights_or_its_pages,
    ),
    ("a block read returns the block that was asked for", |_| {
        storage::a_block_read_returns_the_block_that_was_asked_for()
    }),
    (
        "a program reads served time from a page it cannot write",
        |_| clocks::a_program_reads_served_time_from_a_page_it_cannot_write(),
    ),
    (
        "a completion cannot widen what was offered",
        handles::a_completion_cannot_widen_what_was_offered,
    ),
    (
        "an operation completes after its deadline and only once",
        rings::an_operation_completes_after_its_deadline_and_only_once,
    ),
    (
        "every submission gets exactly one completion",
        rings::every_submission_gets_exactly_one_completion,
    ),
    ("the loader refuses what it may not map", |_| {
        processes::the_loader_refuses_what_it_may_not_map()
    }),
    ("thread local storage follows the thread", |_| {
        threads::thread_local_storage_follows_the_thread()
    }),
    ("a bound space is not reclaimed", |_| {
        address_spaces::a_bound_space_is_not_reclaimed()
    }),
    ("a user page comes back cleared", |_| {
        address_spaces::a_user_page_comes_back_cleared()
    }),
    (
        "a dead program gives its memory back",
        processes::a_dead_program_gives_its_memory_back,
    ),
    (
        "a program drives its own ring",
        processes::a_program_drives_its_own_ring,
    ),
    (
        "an active unmap exposes only its replacement",
        address_spaces::an_active_unmap_exposes_only_its_replacement,
    ),
    (
        "a handle reveals no foreign occupancy",
        handles::a_handle_reveals_no_foreign_occupancy,
    ),
    ("a refused mapping leaves no trace", |_| {
        address_spaces::a_refused_mapping_leaves_no_trace()
    }),
    ("a shared mapping keeps its frame", |_| {
        address_spaces::a_shared_mapping_keeps_its_frame()
    }),
    ("a user space cannot reach the kernel half", |_| {
        address_spaces::a_user_space_cannot_reach_the_kernel_half()
    }),
    ("device mappings refuse allocatable ram", |_| {
        address_spaces::device_mappings_refuse_allocatable_ram()
    }),
    (
        "callee-saved registers survive a switch",
        threads::registers_survive_a_switch,
    ),
    (
        "an exited slot is reused",
        threads::an_exited_slot_is_reused,
    ),
    (
        "a spinning thread cannot starve another",
        threads::a_spinner_cannot_starve_another,
    ),
    ("the clock never runs backwards", |_| {
        clocks::the_clock_never_runs_backwards()
    }),
    (
        "a device write into an unmapped frame faults and changes nothing",
        |_| storage::a_device_write_into_an_unmapped_frame_faults_and_changes_nothing(),
    ),
    ("an nvme timeout quiesces before dma memory returns", |_| {
        storage::an_nvme_timeout_quiesces_before_dma_memory_returns()
    }),
];

fn runs_until(ms: u64, done: impl Fn() -> bool) -> bool {
    if !threads::the_clock_is_running() {
        return false;
    }
    let deadline = time::uptime_ms() + ms;
    loop {
        if !time::is_calibrated() {
            return false;
        }
        if done() {
            return true;
        }
        if time::uptime_ms() >= deadline {
            return false;
        }
        spin_loop();
    }
}

pub(crate) fn run(context: &mut ThreadContext) {
    info!("selftest: image marker v1");
    let mut failed = 0;
    for (name, check) in CHECKS {
        if check(context) {
            info!("selftest: {name}");
        } else {
            failed += 1;
            error!("selftest: {name} FAILED");
        }
    }
    let expected = CHECKS.len();
    let verdict = Verdict {
        passed: expected - failed,
        failed,
        expected,
    };
    info!("{}", Record::Verdict(verdict));
    debug_exit::report(debug_exit::Verdict::from_passed(failed == 0));
}
