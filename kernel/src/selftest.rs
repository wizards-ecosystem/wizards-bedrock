//! Boot-time self-checks.
//!
//! Hardware-dependent checks run on a fully booted `selftest` kernel and report through
//! [`crate::arch::current::debug_exit`]. Pure logic belongs in a host `libs/` test instead. Boot
//! assertions are not repeated because a panic is a missing verdict, not a red check.

use core::hint::spin_loop;
use core::sync::atomic::Ordering;

use wost::arch::current::cpu::Protections;
use wost::arch::current::paging;
use wost::arch::current::{apic, interrupt, pic, segment};
use wost::mm;

use crate::arch::current::debug_exit;
use crate::arch::current::ioapic;
use crate::arch::current::trap;
use crate::arch::current::uart::UartError;
use crate::arch::current::vector;
use crate::pci;
use crate::{dev, error, info, input, thread, time};

macro_rules! handle_table_or_false {
    ($quota:expr) => {{
        let Ok(table) = HandleTable::new($quota) else {
            return false;
        };
        table
    }};
}

macro_rules! shared_handle_table_or_false {
    ($quota:expr) => {
        Arc::new(handle_table_or_false!($quota))
    };
}

mod address_spaces;
mod buses;
mod clocks;
mod files;
mod handles;
mod machine;
mod memory;
mod processes;
mod rings;
mod storage;
mod threads;

use address_spaces::{
    a_boot_mapping_cannot_claim_ownership, a_boot_mapping_cannot_publish_kernel_memory,
    a_bound_space_is_not_reclaimed, a_differing_remap_is_refused,
    a_noncurrent_task_stack_refuses_a_lower_leaf_substitution,
    a_paired_root_admits_only_user_leaves_below_the_split, a_reclaimed_space_answers_nothing,
    a_refused_external_clear_withholds_its_run, a_refused_mapping_leaves_no_trace,
    a_second_space_holds_the_kernel_and_nothing_else,
    a_shared_kernel_unmap_evicts_the_live_user_root, a_shared_mapping_keeps_its_frame,
    a_user_mapping_cannot_widen_a_supervisor_table, a_user_page_comes_back_cleared,
    a_user_space_cannot_reach_the_kernel_half, active_device_tables_cannot_be_reclaimed,
    active_kernel_topology_is_frozen, an_active_unmap_exposes_only_its_replacement,
    an_inactive_lower_root_unmap_preserves_the_live_observer,
    an_undescribed_range_cannot_be_mapped, an_unmapped_range_returns_its_frames,
    binding_ignores_only_accessed_root_drift, caller_global_is_removed_from_safe_mappings,
    device_liveness_covers_the_current_root_and_extent, device_mappings_are_physically_exclusive,
    device_mappings_refuse_allocatable_ram, device_window_reuses_arbitrary_order_gaps,
    external_user_bytes_are_zeroed_before_free, kernel_memory_cannot_be_published_to_ring_three,
    malformed_device_extents_are_refused, one_space_has_one_binding,
    only_a_space_that_matches_the_kernel_can_be_installed,
    paired_space_refusal_and_retirement_conserve_every_frame,
    protected_stacks_refuse_overlapping_maps, ram_mappings_keep_the_direct_map_cache_type,
    shared_mappings_refuse_cache_aliases, shared_topology_refuses_hostile_root_entries,
    supervisor_writes_obey_read_only_pages, the_boot_stack_refuses_a_lower_leaf_substitution,
    the_boot_thread_stands_on_a_guarded_stack, the_direct_map_is_not_executable,
    the_double_fault_stack_cannot_be_unmapped, the_double_fault_stack_is_guarded,
    the_double_fault_stack_refuses_a_lower_leaf_substitution, the_entry_enclave_guards_are_absent,
    the_image_obeys_write_xor_execute, the_last_canonical_page_is_never_user_memory,
    the_loaded_table_is_ours, the_user_root_contains_only_the_entry_enclave,
    two_spaces_map_one_address_to_different_memory, unmapping_refuses_what_it_does_not_own,
    writable_executable_mappings_leave_no_trace,
};
pub(crate) use buses::prepare_pci_claim_checks;
use buses::{
    a_claimed_devices_io_window_is_not_claimable_bare,
    a_device_is_claimed_once_and_a_refusal_strands_nothing, the_bus_walk_descends_through_a_bridge,
};
use clocks::{
    a_program_sees_the_time_page_and_cannot_write_it, ping_arrives, rates_are_believable,
    sweep_clock, the_time_page_and_the_kernel_clock_tell_the_same_time,
    the_time_page_states_the_clock_the_kernel_measured, the_timer_fires_about_once_a_slice,
};
use files::{
    a_device_is_not_a_file, a_file_cannot_be_walked_through,
    a_file_longer_than_a_cluster_is_read_across_its_chain, a_path_resolves_from_a_directory_handle,
    a_second_backend_answers_the_same_three_operations, no_backend_lets_a_path_escape_or_cross,
    no_path_can_name_anything_above_the_handle, the_kernel_reads_the_file_the_host_wrote,
    the_volume_mounts_and_says_what_a_fat32_volume_says,
};
use handles::{
    a_completion_cannot_widen_what_was_offered, a_completion_carries_a_handle_with_narrowed_rights,
    a_dead_table_releases_its_objects, a_full_table_refuses_a_handle_operation_at_submission,
    a_handle_reveals_no_foreign_occupancy, a_refused_handle_operation_returns_its_promise,
    a_table_cannot_exceed_its_quota, an_object_lives_exactly_as_long_as_its_handles,
    filling_the_arena_never_repeats_a_slot, table_identities_never_wrap_or_repeat,
};
use machine::{
    MASKED, a_claim_covers_the_width_it_drives, a_save_stays_inside_its_area,
    default_deny_interrupt_remapping_guards_pci, ports_are_exclusive,
    the_fast_syscall_doors_are_shut_by_this_kernel,
};
use memory::{
    a_run_is_contiguous_and_aligned, allocation_is_zeroed, dirty_memory_can_be_zeroed,
    dma_byte_access_is_exact_and_bounded, frames_are_exclusive, heap_memory_holds_what_it_is_given,
    uncertain_dma_memory_is_never_recycled,
};
use processes::{
    SELFTEST_STARTED_NS, a_dead_program_gives_its_memory_back, a_faulting_program_dies_alone,
    a_faulting_user_copy_resumes_and_reports, a_full_buffer_nvme_read_is_measurable,
    a_meaningless_right_is_refused, a_noncanonical_user_return_stack_retires_only_its_program,
    a_program_cannot_aim_at_the_copy_window, a_program_cannot_bring_smap_down_with_it,
    a_program_drives_its_own_ring, a_program_duplicates_its_bootstrap_handle,
    a_retirement_refusal_is_settled_before_slot_reuse, a_ring_enter_round_trip_is_measurable,
    a_second_thread_crosses_onto_its_own_stack, a_spawned_program_holds_exactly_one_handle,
    a_task_release_refusal_retains_the_zombie_for_retry,
    a_thread_crosses_into_ring_three_and_calls_back, an_uncalibrated_clock_admits_no_user_or_timer,
    an_unguarded_boot_stack_closes_the_user_boundary,
    an_unserved_call_is_refused_without_stopping_the_machine,
    bytes_cross_the_boundary_only_where_they_may, cleanup_refusals_retain_reachable_tables,
    every_refused_spawn_returns_every_resource, guard_rollback_refusal_retains_the_stack,
    missing_extended_state_closes_the_user_boundary,
    no_execute_protection_closes_the_user_boundary, one_zombie_has_one_settlement_collector,
    partial_stack_rollback_is_retryable, report_baseline,
    shared_kernel_mappings_without_proof_close_the_user_boundary,
    the_loader_refuses_what_it_may_not_map, the_return_to_ring_three_clears_buffers,
    unadmitted_interrupts_close_the_user_boundary,
    unsafe_user_return_flags_retire_only_their_program,
};
use rings::{
    a_cancelled_operation_and_its_cancellation_complete_separately,
    a_completion_carries_a_file_handle,
    a_delayed_read_preserves_interrupt_admission_and_exact_teardown,
    a_promise_outlives_the_program_that_made_it, a_refused_settlement_retains_every_owner,
    a_rewritten_submission_cannot_change_what_was_served,
    a_ring_announces_its_shape_and_keeps_its_pages,
    a_ring_operation_cannot_reach_past_its_rights_or_its_pages,
    a_word_that_does_not_fit_is_refused_rather_than_truncated,
    admission_refuses_before_the_completion_queue_can_overflow,
    an_operation_completes_after_its_deadline_and_only_once,
    every_submission_gets_exactly_one_completion, opening_a_node_does_not_restore_universal_rights,
    work_in_flight_is_ended_before_the_ring_is_released,
};
use storage::{
    a_block_read_returns_the_block_that_was_asked_for,
    a_device_write_into_an_unmapped_frame_faults_and_changes_nothing,
    a_domain_proof_is_refused_while_translation_is_off,
    a_failed_bus_master_admission_stops_before_nvme_start,
    a_proven_vtd_revocation_releases_only_its_mapping_owner,
    a_rolled_back_vtd_mapping_releases_its_mapping_owner,
    an_nvme_timeout_quiesces_before_dma_memory_returns,
    an_uncertain_vtd_mapping_never_recycles_its_frames,
    an_uncertain_vtd_unmap_never_recycles_its_frames,
    every_shared_vtd_requester_was_fenced_across_controller_fallback,
    firmware_reserved_dma_regions_never_become_mapping_authority,
    invalid_nvme_namespace_stops_before_io_queues, invalid_nvme_reads_stop_before_submission,
    nvme_candidates_preserve_boot_stack_headroom, the_controller_was_read_rather_than_assumed,
    the_controllers_registers_are_uncacheable,
    the_dma_domain_reaches_exactly_the_live_queue_frames,
    the_queues_wrap_and_the_phase_tag_still_works, translation_is_active_for_the_storage_function,
};
use threads::{
    a_busy_machine_reaps_a_program_without_another_spawn, a_spinner_cannot_starve_another,
    a_stack_stops_being_installed_when_it_is_unmapped,
    a_waiter_rereads_its_condition_before_blocking, an_exited_slot_is_reused,
    every_enabled_state_component_survives_a_switch, registers_survive_a_switch,
    stack_liveness_covers_the_current_root_and_extent, the_clock_is_running,
    thread_local_storage_follows_the_thread, thread_stacks_are_guarded, threads_alternate_strictly,
};

fn runs_until(ms: u64, done: impl Fn() -> bool) -> bool {
    if !the_clock_is_running() {
        return false;
    }
    let deadline = time::uptime_ms() + ms;
    runs_until_deadline(deadline, done)
}

fn runs_until_deadline(deadline: u64, done: impl Fn() -> bool) -> bool {
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

/// Tally of what has been checked so far.
struct Report {
    passed: usize,
    failed: usize,
}

/// Bumped whenever the machine-to-harness verdict grammar changes.
const VERDICT_VERSION: usize = 1;

/// Exact check count; mismatch catches deletion or bypass before the verdict.
const EXPECTED_CHECKS: usize = 211;

#[cfg(feature = "selftest")]
fn awaiting(marker: &str, press: &str) {
    info!("{marker}");
    dev::with_console(|console| {
        console.write_line("");
        console.write_line(press);
        console.write_line("");
    });
}

impl Report {
    const fn new() -> Self {
        Self {
            passed: 0,
            failed: 0,
        }
    }

    fn check(&mut self, name: &str, holds: bool) {
        if holds {
            self.passed += 1;
            info!("selftest: {name}");
        } else {
            self.failed += 1;
            error!("selftest: {name} FAILED");
            if time::Revocation::current() != time::Revocation::None {
                let (tsc, pit, expected) = time::last_watchdog_sample();
                error!(
                    "selftest: clock revoked {:?}, watchdog tsc={tsc} pit={pit} expected={expected}",
                    time::Revocation::current()
                );
            }
        }
    }
}

/// Run every check and hand the verdict to the harness.
pub(crate) fn run(serial: Result<(), UartError>) {
    info!("selftest: image marker v1");
    SELFTEST_STARTED_NS.store(time::now_ns(), Ordering::Relaxed);
    let mut report = Report::new();
    let geometry = dev::console_geometry();

    report.check("serial console came up", serial.is_ok());
    report.check("framebuffer console came up", geometry.is_some());

    report.check(
        "a claimed port cannot be claimed twice",
        ports_are_exclusive(),
    );
    report.check(
        "a save stays inside its area",
        a_save_stays_inside_its_area(),
    );
    report.check(
        "filling the arena never repeats a slot",
        filling_the_arena_never_repeats_a_slot(),
    );
    report.check(
        "ring three has no fast-syscall door",
        the_fast_syscall_doors_are_shut_by_this_kernel(),
    );
    report.check(
        "a reclaimed space answers nothing",
        a_reclaimed_space_answers_nothing(),
    );
    report.check(
        "a claim covers the width it drives",
        a_claim_covers_the_width_it_drives(),
    );

    report.check(
        "the frame allocator admits only exclusively usable frames",
        mm::free_frames() > 0 && mm::overlapping_allocator_claims_are_arbitrated_for_test(),
    );
    report.check(
        "a mapped dma owner drop retains its allocation",
        mm::dma_mapping_owner_drop_retains_for_test(),
    );
    report.check(
        "published iommu table drops retain their allocations",
        mm::published_iommu_table_drop_retains_for_test(),
    );
    report.check(
        "uncertain dma memory is never recycled",
        uncertain_dma_memory_is_never_recycled(),
    );
    report.check(
        "dma byte access is exact and bounded",
        dma_byte_access_is_exact_and_bounded(),
    );
    report.check(
        "two live frames are different frames",
        frames_are_exclusive(),
    );
    report.check(
        "dirty memory can be zeroed, and seen to be",
        dirty_memory_can_be_zeroed(),
    );
    report.check("allocation returns a zeroed frame", allocation_is_zeroed());
    report.check(
        "a contiguous run is contiguous and aligned",
        a_run_is_contiguous_and_aligned(),
    );

    report.check("the loaded page table is ours", the_loaded_table_is_ours());
    report.check(
        "the kernel image obeys write-xor-execute",
        the_image_obeys_write_xor_execute(),
    );
    report.check(
        "the entry enclave guards are absent from the full root",
        the_entry_enclave_guards_are_absent(),
    );
    report.check(
        "supervisor writes obey read-only pages",
        supervisor_writes_obey_read_only_pages(),
    );
    report.check(
        "the direct map is writable and never executable",
        the_direct_map_is_not_executable(),
    );
    report.check(
        "an unserved call is refused without stopping the machine",
        an_unserved_call_is_refused_without_stopping_the_machine(),
    );
    report.check(
        "a thread crosses into ring three and calls back",
        a_thread_crosses_into_ring_three_and_calls_back(),
    );
    report.check(
        "ring-three entry restores the kernel stack selector",
        trap::kernel_stack_selector_on_last_syscall() == u64::from(segment::KERNEL_DATA.bits()),
    );
    report.check(
        "a program cannot bring smap down with it",
        a_program_cannot_bring_smap_down_with_it(),
    );
    report.check(
        "a faulting program dies alone",
        a_faulting_program_dies_alone(),
    );
    report.check(
        "a noncanonical user return stack retires only its program",
        a_noncanonical_user_return_stack_retires_only_its_program(),
    );
    report.check(
        "unsafe user return flags retire only their program",
        unsafe_user_return_flags_retire_only_their_program(),
    );
    report.check(
        "a program cannot aim at the copy window",
        a_program_cannot_aim_at_the_copy_window(),
    );
    report.check(
        "two spaces map one address to different memory",
        two_spaces_map_one_address_to_different_memory(),
    );
    report.check(
        "every enabled state component survives a switch",
        every_enabled_state_component_survives_a_switch(),
    );
    report.check(
        "a ring announces its shape and keeps its pages",
        a_ring_announces_its_shape_and_keeps_its_pages(),
    );
    report.check(
        "a word that does not fit is refused rather than truncated",
        a_word_that_does_not_fit_is_refused_rather_than_truncated(),
    );
    report.check(
        "a completion carries a file handle",
        a_completion_carries_a_file_handle(),
    );
    report.check(
        "opening a node does not restore universal rights",
        opening_a_node_does_not_restore_universal_rights(),
    );
    report.check(
        "a delayed read preserves interrupt admission and exact teardown",
        a_delayed_read_preserves_interrupt_admission_and_exact_teardown(),
    );
    report.check(
        "a ring operation cannot reach past its rights or its pages",
        a_ring_operation_cannot_reach_past_its_rights_or_its_pages(),
    );
    report.check(
        "a second backend answers the same three operations",
        a_second_backend_answers_the_same_three_operations(),
    );
    report.check("a device is not a file", a_device_is_not_a_file());
    report.check(
        "no backend lets a path escape or cross",
        no_backend_lets_a_path_escape_or_cross(),
    );
    report.check(
        "a path resolves from a directory handle",
        a_path_resolves_from_a_directory_handle(),
    );
    report.check(
        "no path can name anything above the handle",
        no_path_can_name_anything_above_the_handle(),
    );
    report.check(
        "a file cannot be walked through",
        a_file_cannot_be_walked_through(),
    );
    report.check(
        "the volume mounts and says what a fat32 volume says",
        the_volume_mounts_and_says_what_a_fat32_volume_says(),
    );
    report.check(
        "the kernel reads the file the host wrote",
        the_kernel_reads_the_file_the_host_wrote(),
    );
    report.check(
        "a file longer than a cluster is read across its chain",
        a_file_longer_than_a_cluster_is_read_across_its_chain(),
    );
    report.check(
        "the controller was read rather than assumed",
        the_controller_was_read_rather_than_assumed(),
    );
    report.check(
        "nvme candidates preserve two pages of boot-stack headroom",
        nvme_candidates_preserve_boot_stack_headroom(),
    );
    report.check(
        "a live nvme bar obeys maxphyaddr",
        crate::a_live_nvme_bar_obeyed_maxphyaddr(),
    );
    report.check(
        "a failed bus master admission stops before nvme start",
        a_failed_bus_master_admission_stops_before_nvme_start(),
    );
    report.check(
        "invalid nvme namespace stops before io queues",
        invalid_nvme_namespace_stops_before_io_queues(),
    );
    report.check(
        "invalid nvme reads stop before submission",
        invalid_nvme_reads_stop_before_submission(),
    );
    report.check(
        "a block read returns the block that was asked for",
        a_block_read_returns_the_block_that_was_asked_for(),
    );
    report.check(
        "the queues wrap and the phase tag still works",
        the_queues_wrap_and_the_phase_tag_still_works(),
    );
    report.check(
        "the controller's registers are uncacheable",
        the_controllers_registers_are_uncacheable(),
    );
    report.check(
        "the bus walk descends through a bridge",
        the_bus_walk_descends_through_a_bridge(),
    );
    report.check(
        "a repeated bridge secondary cannot hide a later bus",
        pci::repeated_secondaries_do_not_hide_a_later_bus_for_test(),
    );
    report.check(
        "a device is claimed once and a refusal strands nothing",
        a_device_is_claimed_once_and_a_refusal_strands_nothing(),
    );
    report.check(
        "a claimed device's io window is not claimable bare",
        a_claimed_devices_io_window_is_not_claimable_bare(),
    );
    report.check(
        "work in flight is ended before the ring is released",
        work_in_flight_is_ended_before_the_ring_is_released(),
    );
    report.check(
        "a refused settlement retains every owner",
        a_refused_settlement_retains_every_owner(),
    );
    report.check(
        "a promise outlives the program that made it",
        a_promise_outlives_the_program_that_made_it(),
    );
    report.check(
        "the time page states the clock the kernel measured",
        the_time_page_states_the_clock_the_kernel_measured(),
    );
    report.check(
        "the time page and the kernel clock tell the same time",
        the_time_page_and_the_kernel_clock_tell_the_same_time(),
    );
    report.check(
        "a program sees the time page and cannot write it",
        a_program_sees_the_time_page_and_cannot_write_it(),
    );
    report.check(
        "a completion carries a handle with narrowed rights",
        a_completion_carries_a_handle_with_narrowed_rights(),
    );
    report.check(
        "a completion cannot widen what was offered",
        a_completion_cannot_widen_what_was_offered(),
    );
    report.check(
        "a refused handle operation returns its promise",
        a_refused_handle_operation_returns_its_promise(),
    );
    report.check(
        "a full table refuses a handle operation at submission",
        a_full_table_refuses_a_handle_operation_at_submission(),
    );
    report.check(
        "an operation completes after its deadline and only once",
        an_operation_completes_after_its_deadline_and_only_once(),
    );
    report.check(
        "a cancelled operation and its cancellation complete separately",
        a_cancelled_operation_and_its_cancellation_complete_separately(),
    );
    report.check(
        "every submission gets exactly one completion",
        every_submission_gets_exactly_one_completion(),
    );
    report.check(
        "admission refuses before the completion queue can overflow",
        admission_refuses_before_the_completion_queue_can_overflow(),
    );
    report.check(
        "a rewritten submission cannot change what was served",
        a_rewritten_submission_cannot_change_what_was_served(),
    );
    report.check(
        "kernel memory cannot be published to ring three",
        kernel_memory_cannot_be_published_to_ring_three(),
    );
    report.check(
        "external user bytes are zeroed before free",
        external_user_bytes_are_zeroed_before_free(),
    );
    report.check(
        "a refused external clear withholds its run",
        a_refused_external_clear_withholds_its_run(),
    );
    report.check(
        "only a space that matches the kernel can be installed",
        only_a_space_that_matches_the_kernel_can_be_installed(),
    );
    report.check(
        "the loader refuses what it may not map",
        the_loader_refuses_what_it_may_not_map(),
    );
    report.check(
        "a second thread crosses onto its own stack",
        a_second_thread_crosses_onto_its_own_stack(),
    );
    report.check(
        "thread local storage follows the thread",
        thread_local_storage_follows_the_thread(),
    );
    report.check(
        "device window reuses arbitrary-order gaps",
        device_window_reuses_arbitrary_order_gaps(),
    );
    let device_liveness = device_liveness_covers_the_current_root_and_extent();
    report.check(
        "late device mappings reach earlier user roots",
        device_liveness
            .as_ref()
            .is_some_and(|probe| probe.shared_topology),
    );
    report.check(
        "a stale shared root cannot authenticate a device mapping",
        device_liveness
            .as_ref()
            .is_some_and(|probe| probe.stale_root),
    );
    report.check(
        "a device mapping checks its first leaf",
        device_liveness.as_ref().is_some_and(|probe| probe.first),
    );
    report.check(
        "a device mapping checks its middle leaf",
        device_liveness.as_ref().is_some_and(|probe| probe.middle),
    );
    report.check(
        "a device mapping checks its last leaf",
        device_liveness.as_ref().is_some_and(|probe| probe.last),
    );
    report.check(
        "a device mapping checks its cache policy",
        device_liveness
            .as_ref()
            .is_some_and(|probe| probe.cache_policy),
    );
    report.check(
        "a bound space is not reclaimed",
        a_bound_space_is_not_reclaimed(),
    );
    report.check("one space has one binding", one_space_has_one_binding());
    report.check(
        "binding ignores only accessed root drift",
        binding_ignores_only_accessed_root_drift(),
    );
    report.check(
        "every refused spawn returns every resource",
        every_refused_spawn_returns_every_resource(),
    );
    report.check(
        "cleanup refusals retain reachable tables",
        cleanup_refusals_retain_reachable_tables(),
    );
    report.check(
        "partial stack rollback is retryable",
        partial_stack_rollback_is_retryable(),
    );
    report.check(
        "guard rollback refusal retains the stack",
        guard_rollback_refusal_retains_the_stack(),
    );
    report.check(
        "a retirement refusal is settled before slot reuse",
        a_retirement_refusal_is_settled_before_slot_reuse(),
    );
    report.check(
        "a task release refusal retains the zombie for retry",
        a_task_release_refusal_retains_the_zombie_for_retry(),
    );
    report.check(
        "one zombie has one settlement collector",
        one_zombie_has_one_settlement_collector(),
    );
    report.check(
        "an unguarded boot stack closes the user boundary",
        an_unguarded_boot_stack_closes_the_user_boundary(),
    );
    report.check(
        "unadmitted interrupts cannot be enabled by a user transition",
        unadmitted_interrupts_close_the_user_boundary(),
    );
    report.check(
        "shared kernel mappings without an isolation proof are refused",
        shared_kernel_mappings_without_proof_close_the_user_boundary(),
    );
    report.check(
        "no-execute protection closes the user boundary",
        no_execute_protection_closes_the_user_boundary(),
    );
    report.check(
        "missing extended state closes the user boundary",
        missing_extended_state_closes_the_user_boundary(),
    );
    report.check(
        "an uncalibrated clock admits no user or timer",
        an_uncalibrated_clock_admits_no_user_or_timer(),
    );
    report.check(
        "a user page comes back cleared",
        a_user_page_comes_back_cleared(),
    );
    report.check(
        "a dead program gives its memory back",
        a_dead_program_gives_its_memory_back(),
    );
    report.check(
        "a program drives its own ring",
        a_program_drives_its_own_ring(),
    );
    report.check(
        "a ring enter round trip is measurable",
        a_ring_enter_round_trip_is_measurable(),
    );
    report.check(
        "a program duplicates its bootstrap handle",
        a_program_duplicates_its_bootstrap_handle(),
    );
    report.check(
        "a spawned program holds exactly one handle",
        a_spawned_program_holds_exactly_one_handle(),
    );
    report.check(
        "the return to ring three clears buffers",
        the_return_to_ring_three_clears_buffers(),
    );
    report.check(
        "a faulting user copy resumes and reports",
        a_faulting_user_copy_resumes_and_reports(),
    );
    report.check(
        "bytes cross the boundary only where they may",
        bytes_cross_the_boundary_only_where_they_may(),
    );
    report.check(
        "an active unmap exposes only its replacement",
        an_active_unmap_exposes_only_its_replacement(),
    );
    report.check(
        "a shared kernel unmap evicts the live user root",
        a_shared_kernel_unmap_evicts_the_live_user_root(),
    );
    report.check(
        "an inactive lower-root unmap preserves the live observer",
        an_inactive_lower_root_unmap_preserves_the_live_observer(),
    );
    report.check(
        "a waiter rereads its condition before blocking",
        a_waiter_rereads_its_condition_before_blocking(),
    );
    report.check(
        "a meaningless right is refused",
        a_meaningless_right_is_refused(),
    );
    report.check(
        "a handle reveals no foreign occupancy",
        a_handle_reveals_no_foreign_occupancy(),
    );
    report.check(
        "table identities never wrap or repeat",
        table_identities_never_wrap_or_repeat(),
    );
    report.check(
        "a table cannot exceed its quota",
        a_table_cannot_exceed_its_quota(),
    );
    report.check(
        "an object lives exactly as long as its handles",
        an_object_lives_exactly_as_long_as_its_handles(),
    );
    report.check(
        "a dead table releases its objects",
        a_dead_table_releases_its_objects(),
    );
    report.check(
        "the boot thread stands on a guarded stack",
        the_boot_thread_stands_on_a_guarded_stack(),
    );
    report.check(
        "the double-fault stack is guarded",
        the_double_fault_stack_is_guarded(),
    );
    report.check(
        "the double-fault stack cannot be unmapped",
        the_double_fault_stack_cannot_be_unmapped(),
    );
    report.check(
        "the boot stack rejects a lower-leaf substitution",
        the_boot_stack_refuses_a_lower_leaf_substitution(),
    );
    report.check(
        "a noncurrent task stack rejects a lower-leaf substitution",
        a_noncurrent_task_stack_refuses_a_lower_leaf_substitution(),
    );
    report.check(
        "the double-fault stack rejects a lower-leaf substitution",
        the_double_fault_stack_refuses_a_lower_leaf_substitution(),
    );
    report.check(
        "protected stacks refuse overlapping maps",
        protected_stacks_refuse_overlapping_maps(),
    );
    report.check(
        "a second space holds the kernel and nothing else",
        a_second_space_holds_the_kernel_and_nothing_else(),
    );
    report.check(
        "a refused mapping leaves no trace",
        a_refused_mapping_leaves_no_trace(),
    );
    report.check(
        "a shared mapping keeps its frame",
        a_shared_mapping_keeps_its_frame(),
    );
    report.check(
        "shared mappings reject cache aliases",
        shared_mappings_refuse_cache_aliases(),
    );
    report.check(
        "ram mappings keep the direct map's cache type",
        ram_mappings_keep_the_direct_map_cache_type(),
    );
    report.check(
        "an unmapped range returns its frames",
        an_unmapped_range_returns_its_frames(),
    );
    report.check(
        "unmapping refuses what it does not own",
        unmapping_refuses_what_it_does_not_own(),
    );
    report.check(
        "a boot mapping cannot claim ownership",
        a_boot_mapping_cannot_claim_ownership(),
    );
    report.check(
        "writable executable mappings leave no trace",
        writable_executable_mappings_leave_no_trace(),
    );
    report.check(
        "caller global is removed from safe mappings",
        caller_global_is_removed_from_safe_mappings(),
    );
    report.check(
        "a boot mapping cannot publish kernel memory",
        a_boot_mapping_cannot_publish_kernel_memory(),
    );
    report.check(
        "a paired root admits only user leaves below the split",
        a_paired_root_admits_only_user_leaves_below_the_split(),
    );
    report.check(
        "a user space cannot reach the kernel half",
        a_user_space_cannot_reach_the_kernel_half(),
    );
    report.check(
        "the last canonical page is never user memory",
        the_last_canonical_page_is_never_user_memory(),
    );
    report.check(
        "a user mapping cannot widen a supervisor table",
        a_user_mapping_cannot_widen_a_supervisor_table(),
    );
    report.check(
        "an undescribed range cannot be mapped",
        an_undescribed_range_cannot_be_mapped(),
    );
    report.check(
        "a differing remap is refused",
        a_differing_remap_is_refused(),
    );
    report.check(
        "device mappings are physically exclusive",
        device_mappings_are_physically_exclusive(),
    );
    report.check(
        "device mappings refuse allocatable ram",
        device_mappings_refuse_allocatable_ram(),
    );
    report.check(
        "malformed device extents are refused",
        malformed_device_extents_are_refused(),
    );
    report.check(
        "active device tables cannot be reclaimed",
        active_device_tables_cannot_be_reclaimed(),
    );
    report.check(
        "active kernel topology refuses a new shared root",
        active_kernel_topology_is_frozen(),
    );
    report.check(
        "shared topology rejects hostile root entries",
        shared_topology_refuses_hostile_root_entries(),
    );

    report.check(
        "heap memory holds what it is given",
        heap_memory_holds_what_it_is_given(),
    );

    report.check("threads are available", thread::is_available());
    report.check(
        "an unmapped stack is no longer installed",
        a_stack_stops_being_installed_when_it_is_unmapped(),
    );
    let stack_liveness = stack_liveness_covers_the_current_root_and_extent();
    report.check(
        "a stale shared root cannot authenticate a stack",
        stack_liveness
            .as_ref()
            .is_some_and(|probe| probe.stale_root),
    );
    report.check(
        "a stack checks its first leaf",
        stack_liveness.as_ref().is_some_and(|probe| probe.first),
    );
    report.check(
        "a stack checks its middle leaf",
        stack_liveness.as_ref().is_some_and(|probe| probe.middle),
    );
    report.check(
        "a stack checks its last leaf",
        stack_liveness.as_ref().is_some_and(|probe| probe.last),
    );
    report.check(
        "thread stacks are guarded below",
        thread_stacks_are_guarded(),
    );
    report.check(
        "two threads alternate strictly through yields",
        threads_alternate_strictly(),
    );
    report.check(
        "callee-saved registers survive a switch",
        registers_survive_a_switch(),
    );
    report.check("an exited slot is reused", an_exited_slot_is_reused());
    report.check(
        "a spinning thread cannot starve another",
        a_spinner_cannot_starve_another(),
    );

    report.check(
        "the interrupt table the CPU holds is ours",
        interrupt::table_is_loaded(),
    );

    let before = trap::last_breakpoint();
    interrupt::breakpoint();
    let taken = trap::last_breakpoint();

    report.check(
        "a breakpoint is delivered and resumes",
        taken.count == before.count + 1,
    );

    report.check(
        "every protection the CPU supports is enabled",
        Protections::enabled() == Protections::supported(),
    );

    report.check(
        "the legacy controllers are masked",
        pic::masks() == Some((MASKED, MASKED)),
    );
    report.check(
        "local apic initialization stayed unpublished and interrupt-excluded",
        apic::initialization_was_atomic_for_test(),
    );
    report.check("this cpu is accepting interrupts", interrupt::are_enabled());

    report.check(
        "both calibrated rates are believable",
        rates_are_believable(),
    );

    let sweep = sweep_clock();
    report.check("the clock never runs backwards", sweep.monotonic);
    report.check(
        "the timer fires about once a slice under load",
        the_timer_fires_about_once_a_slice(),
    );

    thread::start_idle();
    report.check(
        "a busy machine reaps a program without another spawn",
        a_busy_machine_reaps_a_program_without_another_spawn(),
    );
    report.check(
        "the user root contains only the entry enclave",
        the_user_root_contains_only_the_entry_enclave(),
    );
    report.check(
        "every user entry reaches Rust on the full root",
        wost::arch::current::kpti_every_user_entry_reached_rust_on_full_root_for_test(),
    );
    report.check(
        "every user return installs the stripped root",
        wost::arch::current::kpti_every_user_return_installed_stripped_root_for_test(),
    );
    report.check(
        "the entry stack is guarded and scrubbed",
        wost::arch::current::kpti_entry_stacks_are_guarded_and_scrubbed_for_test(),
    );
    report.check(
        "a context switch moves one complete root pair",
        wost::arch::current::kpti_context_switch_moved_complete_root_pairs_for_test(),
    );
    report.check(
        "paired-space refusal and retirement conserve every frame",
        paired_space_refusal_and_retirement_conserve_every_frame(),
    );

    input::start_poller();
    report.check("the keyboard controller came up", input::is_ready());
    report.check(
        "a refused keyboard route restores configuration, masking and mappings",
        crate::keyboard_routing_rollback_is_exact()
            && ioapic::ignored_writes_are_refused_for_test()
            && crate::unproven_external_controllers_veto_interrupts(),
    );

    // Kept in step with READY_MARKER in `xtask/src/qemu.rs`; nothing may talk
    // to the keyboard between the marker and the wait.
    let first_key = input::arm_key(ps2_keys::Key::A);
    let last_key = input::arm_key(ps2_keys::Key::Enter);
    awaiting(
        "selftest: awaiting injected keystroke",
        "  PRESS  a  then BACKSPACE  then ENTER",
    );
    report.check(
        "an injected keystroke arrives, and its release follows",
        first_key.is_some_and(input::await_armed_key),
    );

    report.check(
        "default-deny interrupt remapping guards every pci requester",
        default_deny_interrupt_remapping_guards_pci(),
    );
    // Before the editing check: Enter is the last injected key and only a
    // press echoes, so its release is when the console stops moving.
    report.check(
        "the injected editing keys arrive, ending with enter",
        last_key.is_some_and(input::await_armed_key),
    );

    // The set 1 path: QEMU honours the translation bit, producing the stream VMware delivers
    // unasked, with a key the harness chose.
    report.check(
        "the controller accepts translation being turned on",
        input::enable_translation(),
    );
    // Seeded against the stream on purpose: the round can succeed only by reading the bytes, as
    // VMware forces.
    input::assume_wrong_numbering();
    // Kept in step with TRANSLATED_MARKER in `xtask/src/qemu.rs`, under the
    // same rule: nothing talks to the keyboard between marker and wait.
    let translated_key = input::arm_key(ps2_keys::Key::B);
    awaiting(
        "selftest: awaiting translated keystroke",
        "  PRESS  b   for the last round",
    );
    // `B`, not `A`, so the first round's stale event cannot satisfy this wait.
    report.check(
        "a translated keystroke decodes to the same key",
        translated_key.is_some_and(input::await_armed_key),
    );

    report.check(
        "no spurious interrupt has been delivered",
        trap::spurious_count() == 0,
    );

    // Last, because reading the error register clears it.
    report.check(
        "the local apic has recorded no error",
        apic::error_status() == 0,
    );

    report.check(
        "a full-buffer nvme read is measurable",
        a_full_buffer_nvme_read_is_measurable(),
    );

    report.check(
        "translation is active for the storage function",
        translation_is_active_for_the_storage_function(),
    );
    report.check(
        "every shared vtd requester was fenced across controller fallback",
        every_shared_vtd_requester_was_fenced_across_controller_fallback(),
    );
    report.check(
        "the dma domain reaches exactly the live queue frames",
        the_dma_domain_reaches_exactly_the_live_queue_frames(),
    );
    // Before the translation toggle below, which flushes the unit's caches
    // and would hide a stale translation this check exists to expose.
    report.check(
        "a device write into an unmapped frame faults and changes nothing",
        a_device_write_into_an_unmapped_frame_faults_and_changes_nothing(),
    );
    report.check(
        "a proven vtd revocation releases only its mapping owner",
        a_proven_vtd_revocation_releases_only_its_mapping_owner(),
    );
    report.check(
        "a rolled back vtd mapping releases its mapping owner",
        a_rolled_back_vtd_mapping_releases_its_mapping_owner(),
    );
    report.check(
        "uncertain vtd mapping never recycles its frames",
        an_uncertain_vtd_mapping_never_recycles_its_frames(),
    );
    report.check(
        "uncertain vtd unmapping never recycles its frames",
        an_uncertain_vtd_unmap_never_recycles_its_frames(),
    );
    report.check(
        "a domain proof is refused while translation is off",
        a_domain_proof_is_refused_while_translation_is_off(),
    );
    report.check(
        "firmware reserved dma regions never become mapping authority",
        firmware_reserved_dma_regions_never_become_mapping_authority(),
    );

    // Last of all: the timeout proof resets the only storage controller.
    report.check(
        "an nvme timeout quiesces before dma memory returns",
        an_nvme_timeout_quiesces_before_dma_memory_returns(),
    );

    report.check(
        "a pending local interrupt refuses the first sti",
        apic::pending_audit_refuses_queued_self_interrupt_for_test(|| trap::last_ping().count),
    );

    // Last interrupt exercise: a broken EOI cannot starve later timed checks.
    let before = trap::last_ping();
    let first_sent = apic::send_self_interrupt().is_ok();
    report.check(
        "a self-interrupt is delivered",
        first_sent && ping_arrives(before.count + 1),
    );

    let second_sent = apic::send_self_interrupt().is_ok();
    report.check(
        "the interrupt is dismissed, so the next one arrives",
        second_sent && ping_arrives(before.count + 2),
    );

    let apic_failures =
        apic::initialization_failures_for_test(vector::LOCAL_APIC, || trap::last_ping().count);
    report.check(
        "every late or ignored-write apic refusal stays unavailable and terminal",
        apic_failures.unavailable_and_terminal()
            & apic_failures.ignored_spurious_write_is_terminal(),
    );
    report.check(
        "apic takeover masks exactly every advertised local source",
        apic_failures.advertised_sources_masked(),
    );
    report.check(
        "unmaskable or unknown apic source inventories poison takeover",
        apic_failures.source_mask_refusals_are_terminal(),
    );

    let activation = paging::activation_publication_for_test();
    report.check(
        "paging activation stays unpublished under interrupt exclusion",
        activation.initializing_is_excluded(),
    );
    report.check(
        "paging activation publishes its root before readiness",
        activation.root_precedes_ready(),
    );

    report_baseline();

    let checked = report.passed + report.failed;
    let passed = report.failed == 0 && checked == EXPECTED_CHECKS && EXPECTED_CHECKS > 0;
    if passed {
        info!(
            "selftest: verdict v{} PASS passed={} failed={} expected={}",
            VERDICT_VERSION, report.passed, report.failed, EXPECTED_CHECKS
        );
    } else {
        error!(
            "selftest: verdict v{} FAIL passed={} failed={} expected={}",
            VERDICT_VERSION, report.passed, report.failed, EXPECTED_CHECKS
        );
    }
    debug_exit::report(debug_exit::Verdict::from_passed(passed));
}
