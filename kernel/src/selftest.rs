//! Boot-time self-checks.
//!
//! Hardware-dependent checks run on a fully booted `selftest` kernel and report
//! through [`crate::arch::current::debug_exit`]. Pure logic belongs in a host
//! `libs/` test instead. Boot assertions are not repeated because a panic is a
//! missing verdict, not a red check.

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
use crate::{dev, error, info, input, mem, thread, time, warn};

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
    a_large_space_returns_every_table_frame, a_mapping_added_after_the_switch_is_live,
    a_noncurrent_task_stack_rejects_a_lower_leaf_substitution,
    a_paired_root_admits_only_user_leaves_below_the_split,
    a_partial_mapping_failure_returns_its_tables, a_reclaimed_space_answers_nothing,
    a_refused_external_clear_withholds_its_run, a_refused_mapping_leaves_no_trace,
    a_refused_remap_preserves_every_older_entry, a_second_space_holds_the_kernel_and_nothing_else,
    a_second_switch_is_refused, a_shared_kernel_unmap_evicts_the_live_user_root,
    a_shared_mapping_keeps_its_frame, a_user_mapping_cannot_widen_a_supervisor_table,
    a_user_page_comes_back_cleared, a_user_space_cannot_reach_the_kernel_half,
    a_walk_reports_the_permission_the_cpu_grants, active_device_tables_cannot_be_reclaimed,
    active_kernel_topology_is_frozen, an_active_unmap_exposes_only_its_replacement,
    an_admitted_canonical_page_can_be_invalidated,
    an_inactive_lower_root_unmap_preserves_the_live_observer,
    an_undescribed_range_cannot_be_mapped, an_unmapped_range_returns_its_frames,
    binding_ignores_only_accessed_root_drift, caller_global_is_removed_from_safe_mappings,
    device_liveness_covers_the_current_root_and_extent, device_mappings_are_physically_exclusive,
    device_mappings_refuse_allocatable_ram, device_window_reuses_arbitrary_order_gaps,
    external_user_bytes_are_zeroed_before_free, firmware_reads_are_bounded,
    kernel_memory_cannot_be_published_to_ring_three, malformed_device_extents_are_refused,
    one_space_has_one_binding, only_a_space_that_matches_the_kernel_can_be_installed,
    paired_space_refusal_and_retirement_conserve_every_frame,
    protected_stacks_refuse_overlapping_maps, ram_mappings_keep_the_direct_map_cache_type,
    shared_mappings_reject_cache_aliases, shared_topology_rejects_hostile_root_entries,
    supervisor_writes_obey_read_only_pages, the_boot_stack_rejects_a_lower_leaf_substitution,
    the_boot_thread_stands_on_a_guarded_stack, the_direct_map_is_not_executable,
    the_double_fault_stack_cannot_be_unmapped, the_double_fault_stack_is_guarded,
    the_double_fault_stack_rejects_a_lower_leaf_substitution, the_entry_enclave_guards_are_absent,
    the_image_obeys_write_xor_execute, the_last_canonical_page_is_never_user_memory,
    the_loaded_table_is_ours, the_map_offers_a_run_to_probe,
    the_user_root_contains_only_the_entry_enclave, two_spaces_map_one_address_to_different_memory,
    unmapping_refuses_what_it_does_not_own, writable_executable_mappings_leave_no_trace,
};
pub(crate) use buses::prepare_pci_claim_checks;
use buses::{
    a_claimed_devices_io_window_is_not_claimable_bare,
    a_device_is_claimed_once_and_a_refusal_strands_nothing, the_bus_walk_descends_through_a_bridge,
};
use clocks::{
    a_program_sees_the_time_page_and_cannot_write_it, a_sleeper_wakes_on_time_and_quietly,
    an_idle_kernel_takes_no_timer_interrupts, ping_arrives, rates_are_believable,
    revoking_the_clock_sends_readers_to_uptime, sleeping_hands_the_cpu_over, sweep_clock,
    the_clocks_update_window_is_never_preemptible,
    the_time_page_and_the_kernel_clock_tell_the_same_time,
    the_time_page_states_the_clock_the_kernel_measured, the_timer_fires_about_once_a_slice,
    timer_entry_is_programmed,
};
use files::{
    a_device_is_not_a_file, a_file_cannot_be_walked_through,
    a_file_longer_than_a_cluster_is_read_across_its_chain, a_path_resolves_from_a_directory_handle,
    a_path_resolves_through_a_directory, a_second_backend_answers_the_same_three_operations,
    enumeration_callbacks_run_without_backend_ownership,
    enumeration_lists_what_traversal_would_find, no_backend_lets_a_path_escape_or_cross,
    no_path_can_name_anything_above_the_handle, the_kernel_reads_the_file_the_host_wrote,
    the_volume_mounts_and_says_what_a_fat32_volume_says,
};
use handles::{
    a_completion_cannot_widen_what_was_offered, a_completion_carries_a_handle_with_narrowed_rights,
    a_dead_table_releases_its_objects, a_full_table_refuses_a_handle_operation_at_submission,
    a_handle_reveals_no_foreign_occupancy, a_refused_handle_operation_returns_its_promise,
    a_table_cannot_exceed_its_quota, an_object_lives_exactly_as_long_as_its_handles,
    filling_the_arena_never_repeats_a_slot, table_identities_never_wrap_or_repeat,
    the_handle_arena_backs_a_tables_whole_quota,
};
use machine::{
    LONGEST_SCROLL, MASKED, TRANSLATED_MF2, UNTRANSLATED_MF2, a_claim_covers_the_width_it_drives,
    a_lock_excludes_interrupts, a_refused_pair_of_claims_strands_neither,
    a_save_stays_inside_its_area, a_scroll_does_not_block_interrupts,
    a_scroll_moves_one_visible_cursor, channels_are_disjoint,
    default_deny_interrupt_remapping_guards_pci,
    forgotten_lock_ownership_is_distinct_from_contention, frame_is_consistent,
    frame_spans_the_saved_words, geometry_is_sane, keyboard_config_matches_its_routing,
    keyboard_input_is_polled_without_legacy_interrupts, latch_admits_one, lock_excludes,
    ping_is_consistent, ports_are_exclusive, the_editing_keys_move_the_cursor,
    the_fast_syscall_doors_are_shut_by_this_kernel, the_framebuffer_is_write_combining,
    the_io_apic_owner_was_returned_after_masked_rollback,
};
use memory::{
    a_large_allocation_takes_and_returns_frames, a_refill_costs_one_frame_and_balances,
    a_run_is_contiguous_and_aligned, allocation_is_zeroed, dirty_memory_can_be_zeroed,
    dma_byte_access_is_exact_and_bounded, frame_round_trips, frames_are_exclusive,
    heap_memory_holds_what_it_is_given, the_heap_reuses_a_freed_slot,
    uncertain_dma_memory_is_never_recycled,
};
use processes::{
    SELFTEST_STARTED_NS, a_clock_disagreement_revokes_time_without_stranding_timers,
    a_commit_refusal_reunites_cleanup_authority, a_dead_program_gives_its_memory_back,
    a_faulting_program_dies_alone, a_faulting_user_copy_resumes_and_reports,
    a_full_buffer_nvme_read_is_measurable, a_maximum_image_and_stack_fit_the_cleanup_ledger,
    a_meaningless_right_is_refused, a_noncanonical_user_return_stack_retires_only_its_program,
    a_program_cannot_aim_at_the_copy_window, a_program_cannot_bring_smap_down_with_it,
    a_program_drives_its_own_ring, a_program_duplicates_its_bootstrap_handle,
    a_program_waits_through_its_trap_frame, a_retirement_refusal_is_settled_before_slot_reuse,
    a_ring_enter_round_trip_is_measurable, a_second_thread_crosses_onto_its_own_stack,
    a_spawned_program_holds_exactly_one_handle, a_syscall_wake_arms_the_timer_for_what_it_woke,
    a_task_release_refusal_retains_the_zombie_for_retry,
    a_thread_crosses_into_ring_three_and_calls_back,
    an_in_flight_collector_keeps_the_worker_retry_live,
    an_uncalibrated_clock_admits_no_user_or_timer,
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
    a_cancelled_open_releases_its_reserved_handle_before_quiet,
    a_cancelled_operation_and_its_cancellation_complete_separately,
    a_completion_carries_a_file_handle,
    a_delayed_read_preserves_interrupt_admission_and_exact_teardown,
    a_full_ring_admits_again_only_after_userspace_reaps,
    a_promise_outlives_the_program_that_made_it, a_read_is_a_submission_with_its_own_offset,
    a_refused_settlement_retains_every_owner, a_rewritten_submission_cannot_change_what_was_served,
    a_ring_announces_its_shape_and_keeps_its_pages,
    a_ring_operation_cannot_reach_past_its_rights_or_its_pages,
    a_word_that_does_not_fit_is_refused_rather_than_truncated,
    admission_refuses_before_the_completion_queue_can_overflow,
    an_operation_completes_after_its_deadline_and_only_once,
    every_submission_gets_exactly_one_completion, file_work_pressure_is_bounded_at_admission,
    opening_a_node_does_not_restore_universal_rights,
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
    the_queues_wrap_and_the_phase_tag_still_works, the_unit_walks_the_kernels_root_table,
    translation_is_active_for_the_storage_function,
};
use threads::{
    a_blocked_reader_waits_while_the_poller_runs,
    a_busy_machine_reaps_a_program_without_another_spawn,
    a_failed_idle_start_can_be_retried_exactly_once, a_leaf_guard_is_the_last_lock,
    a_new_thread_is_interruptible, a_spinner_cannot_starve_another,
    a_stack_stops_being_installed_when_it_is_unmapped, a_thread_starts_aligned,
    a_waiter_rereads_its_condition_before_blocking, an_exited_slot_is_reused,
    every_enabled_state_component_survives_a_switch, registers_survive_a_switch,
    runtime_divides_by_weight, stack_liveness_covers_the_current_root_and_extent,
    the_blocked_reader_woke_with_the_key, the_clock_is_running,
    the_idle_thread_runs_only_when_nothing_else_can,
    the_polled_fallback_runs_its_rounds_and_sleeps, thread_local_storage_follows_the_thread,
    thread_stacks_are_guarded, threads_alternate_strictly,
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
/// Changing it requires a retained mutation (D-119).
const EXPECTED_CHECKS: usize = 296;

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
        "framebuffer geometry is sane",
        geometry.is_some_and(geometry_is_sane),
    );
    report.check(
        "colour channels do not overlap",
        geometry.is_some_and(channels_are_disjoint),
    );

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
        "a refused pair of claims strands neither",
        a_refused_pair_of_claims_strands_neither(),
    );
    report.check(
        "a claim covers the width it drives",
        a_claim_covers_the_width_it_drives(),
    );
    report.check("a held lock excludes another holder", lock_excludes());
    report.check(
        "forgotten lock ownership is distinct from contention",
        forgotten_lock_ownership_is_distinct_from_contention(),
    );
    report.check("a one-shot latch admits one caller", latch_admits_one());

    report.check(
        "the frame allocator admits only exclusively usable frames",
        mm::free_frames() > 0 && mm::overlapping_allocator_claims_are_arbitrated_for_test(),
    );
    report.check(
        "a frame is returned when its handle drops",
        frame_round_trips(),
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
        "an admitted canonical page can be invalidated",
        an_admitted_canonical_page_can_be_invalidated(),
    );
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
        "a mapping added after the switch is live",
        a_mapping_added_after_the_switch_is_live(),
    );
    report.check(
        "a second address space is refused",
        a_second_switch_is_refused(),
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
        "a read is a submission with its own offset",
        a_read_is_a_submission_with_its_own_offset(),
    );
    report.check(
        "a cancelled open releases its reserved handle before quiet",
        a_cancelled_open_releases_its_reserved_handle_before_quiet(),
    );
    report.check(
        "file work pressure is bounded at admission",
        file_work_pressure_is_bounded_at_admission(),
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
    report.check(
        "enumeration callbacks run without backend ownership",
        enumeration_callbacks_run_without_backend_ownership(),
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
        "enumeration lists what traversal would find",
        enumeration_lists_what_traversal_would_find(),
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
        "a path resolves through a directory",
        a_path_resolves_through_a_directory(),
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
        "a later nvme controller survives an earlier failure",
        crate::a_later_nvme_controller_survives_an_earlier_failure(),
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
        "revoking the clock sends readers to ring Uptime",
        revoking_the_clock_sends_readers_to_uptime(),
    );
    report.check(
        "the clock's update window is never preemptible",
        the_clocks_update_window_is_never_preemptible(),
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
        "a full ring admits again only after userspace reaps",
        a_full_ring_admits_again_only_after_userspace_reaps(),
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
        "a commit refusal reunites cleanup authority",
        a_commit_refusal_reunites_cleanup_authority(),
    );
    report.check(
        "a maximum image and stack fit the cleanup ledger",
        a_maximum_image_and_stack_fit_the_cleanup_ledger(),
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
        "a failed boot-space build returns every table frame",
        mem::partial_build_failure_is_atomic_for_test(),
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
        "an in-flight collector keeps the worker retry live",
        an_in_flight_collector_keeps_the_worker_retry_live(),
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
        "a clock disagreement revokes time without stranding timers",
        a_clock_disagreement_revokes_time_without_stranding_timers(),
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
        "a program waits through its trap frame",
        a_program_waits_through_its_trap_frame(),
    );
    report.check(
        "a syscall wake arms the timer for what it woke",
        a_syscall_wake_arms_the_timer_for_what_it_woke(),
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
        "the handle arena backs a table's whole quota",
        the_handle_arena_backs_a_tables_whole_quota(),
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
        the_boot_stack_rejects_a_lower_leaf_substitution(),
    );
    report.check(
        "a noncurrent task stack rejects a lower-leaf substitution",
        a_noncurrent_task_stack_rejects_a_lower_leaf_substitution(),
    );
    report.check(
        "the double-fault stack rejects a lower-leaf substitution",
        the_double_fault_stack_rejects_a_lower_leaf_substitution(),
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
        "a walk reports what the live tables grant",
        a_walk_reports_the_permission_the_cpu_grants(),
    );
    report.check(
        "a refused mapping leaves no trace",
        a_refused_mapping_leaves_no_trace(),
    );
    report.check(
        "a partial mapping failure returns its tables",
        a_partial_mapping_failure_returns_its_tables(),
    );
    report.check(
        "a refused remap preserves every older entry",
        a_refused_remap_preserves_every_older_entry(),
    );
    report.check(
        "a shared mapping keeps its frame",
        a_shared_mapping_keeps_its_frame(),
    );
    report.check(
        "shared mappings reject cache aliases",
        shared_mappings_reject_cache_aliases(),
    );
    report.check(
        "ram mappings keep the direct map's cache type",
        ram_mappings_keep_the_direct_map_cache_type(),
    );
    report.check(
        "a large space returns every table frame",
        a_large_space_returns_every_table_frame(),
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
        "the map offers a run to probe",
        the_map_offers_a_run_to_probe(),
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
        shared_topology_rejects_hostile_root_entries(),
    );
    report.check(
        "firmware reads are confined to acpi storage",
        firmware_reads_are_bounded(),
    );

    report.check(
        "the heap reuses a freed slot",
        the_heap_reuses_a_freed_slot(),
    );
    report.check(
        "a refill costs one frame and balances",
        a_refill_costs_one_frame_and_balances(),
    );
    report.check(
        "heap memory holds what it is given",
        heap_memory_holds_what_it_is_given(),
    );
    report.check(
        "a large allocation takes and returns frames",
        a_large_allocation_takes_and_returns_frames(),
    );
    report.check(
        "the console has a complete shadow to scroll from",
        dev::console_has_complete_back_buffer(),
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
    report.check("a thread starts aligned", a_thread_starts_aligned());
    report.check("an exited slot is reused", an_exited_slot_is_reused());
    report.check(
        "a new thread runs with interrupts accepted",
        a_new_thread_is_interruptible(),
    );
    report.check(
        "a spinning thread cannot starve another",
        a_spinner_cannot_starve_another(),
    );
    report.check(
        "runtime divides in proportion to weight",
        runtime_divides_by_weight(),
    );
    report.check(
        "the policy never needed its fallback",
        thread::policy_fallbacks() == 0,
    );

    report.check(
        "the code selector is the one we installed",
        segment::code_selector() == segment::KERNEL_CODE,
    );
    report.check(
        "the task register holds our segment",
        segment::task_register() == segment::TASK_STATE,
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
        "the saved frame is the frame the CPU pushed",
        frame_is_consistent(taken),
    );
    report.check(
        "interrupt entry pushes the frame the layout describes",
        frame_spans_the_saved_words(taken),
    );

    report.check(
        "every protection the CPU supports is enabled",
        Protections::enabled() == Protections::supported(),
    );

    report.check(
        "the legacy controllers are masked",
        pic::masks() == Some((MASKED, MASKED)),
    );
    report.check("the local apic is software enabled", apic::is_enabled());
    report.check(
        "local apic initialization stayed unpublished and interrupt-excluded",
        apic::initialization_was_atomic_for_test(),
    );
    report.check("this cpu is accepting interrupts", interrupt::are_enabled());
    report.check(
        "a lock excludes interrupts and gives them back",
        a_lock_excludes_interrupts(),
    );

    report.check(
        "the timer entry is one-shot and unmasked",
        timer_entry_is_programmed(),
    );
    report.check(
        "both calibrated rates are believable",
        rates_are_believable(),
    );
    report.check(
        "every refusal the clock can record reads back as itself",
        time::every_refusal_reads_back_as_itself(),
    );

    let sweep = sweep_clock();
    report.check("the clock never runs backwards", sweep.monotonic);
    // Interpolation alone cannot prove two events inside one tick are distinguishable.
    report.check(
        "the clock advances in steps finer than a microsecond",
        sweep.advanced
            && time::is_calibrated()
            && sweep.finest_step.is_some_and(|step| step < 1_000),
    );
    report.check(
        "an idle kernel takes no timer interrupts",
        an_idle_kernel_takes_no_timer_interrupts(),
    );
    report.check(
        "the timer fires about once a slice under load",
        the_timer_fires_about_once_a_slice(),
    );
    report.check(
        "a sleeping thread wakes on time, and quietly",
        a_sleeper_wakes_on_time_and_quietly(),
    );
    report.check(
        "a sleeping thread hands the cpu over",
        sleeping_hands_the_cpu_over(),
    );

    // Everything above ran without an idle thread, the only witness for a
    // sleeper halting on its own stack; from here something is always runnable.
    report.check(
        "a failed idle start can be retried exactly once",
        a_failed_idle_start_can_be_retried_exactly_once(),
    );
    thread::start_idle();
    report.check("the idle thread is running", thread::idle_is_started());
    report.check(
        "the idle thread runs only when nothing else can",
        the_idle_thread_runs_only_when_nothing_else_can(),
    );
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
    report.check(
        "the polled keyboard fallback is bounded before it stays live",
        the_polled_fallback_runs_its_rounds_and_sleeps(),
    );

    report.check("the keyboard controller came up", input::is_ready());
    report.check(
        "a refused controller configuration is typed and changes nothing",
        input::configuration_refusal_is_visible(),
    );
    report.check(
        "a refused keyboard route restores configuration, masking and mappings",
        crate::keyboard_routing_rollback_is_exact()
            && ioapic::ignored_writes_are_refused_for_test()
            && crate::unproven_external_controllers_veto_interrupts(),
    );
    report.check(
        "an isolated I/O APIC route encodes its typed vector",
        ioapic::typed_route_is_encoded_for_test(),
    );
    report.check(
        "a window that is not an I/O APIC is never taken over",
        ioapic::an_implausible_version_is_refused_for_test(),
    );
    report.check(
        "a failed controller flush stops every following transaction",
        input::flush_refusals_are_visible(),
    );
    report.check(
        "an 8042 backlog of exactly the flush bound drains",
        crate::arch::current::i8042::a_backlog_of_the_flush_bound_drains_for_test(),
    );
    report.check(
        "a refused controller command is typed and consumes no reply",
        input::transport_refusals_are_visible(),
    );
    report.check(
        "the controller route and ordered interrupt witness are exact",
        keyboard_config_matches_its_routing()
            && input::watch_order_is_causal()
            && input::queue_admission_is_exact()
            && input::producer_order_is_causal(),
    );
    report.check(
        "the keyboard interrupt handoff records its ownership",
        input::interrupt_handoff_is_recorded(),
    );
    report.check(
        "an echo to the keyboard comes back",
        input::echo_roundtrips(),
    );
    // The identity first, because it decides what the set query is worth;
    // recorded as answered, since `None` and a wrong number differ (D-176).
    let identity = input::identity();
    let translated = identity == Some(TRANSLATED_MF2);
    if !translated && identity != Some(UNTRANSLATED_MF2) {
        warn!("keyboard: identifies as {identity:?}, which names no known keyboard");
    }
    report.check(
        "the keyboard identifies as one the decoder knows",
        translated || identity == Some(UNTRANSLATED_MF2),
    );

    // Translation rewrites set-query replies, so this stays diagnostic and
    // the decoder is proved from received bytes (D-178, D-179, D-239).
    let set = input::scancode_set();
    if !translated && set != Some(2) {
        warn!("keyboard: reports scancode set {set:?}, not 2");
    }

    // Set up before the marker: the reader must be blocked before the harness
    // presses, or an already-waiting keystroke proves nothing.
    report.check(
        "a blocked keyboard reader waits while the poller runs",
        a_blocked_reader_waits_while_the_poller_runs(),
    );

    // Kept in step with READY_MARKER in `xtask/src/qemu.rs`; nothing may talk
    // to the keyboard between the marker and the wait.
    let first_key = input::arm_key(ps2_keys::Key::A);
    let last_key = input::arm_key(ps2_keys::Key::Enter);
    let polling_before = input::poller_passes();
    awaiting(
        "selftest: awaiting injected keystroke",
        "  PRESS  a  then BACKSPACE  then ENTER",
    );
    report.check(
        "an injected keystroke arrives, and its release follows",
        first_key.is_some_and(input::await_armed_key),
    );

    report.check(
        "the framebuffer is mapped write-combining",
        the_framebuffer_is_write_combining(),
    );
    report.check(
        "default-deny interrupt remapping guards every pci requester",
        default_deny_interrupt_remapping_guards_pci(),
    );
    report.check(
        "the io apic owner returned after masked rollback",
        the_io_apic_owner_was_returned_after_masked_rollback(),
    );
    report.check(
        "keyboard input is polled without legacy interrupts",
        keyboard_input_is_polled_without_legacy_interrupts(),
    );
    report.check(
        "the blocked reader woke with the injected key through polling",
        the_blocked_reader_woke_with_the_key(polling_before),
    );
    // Before the editing check: Enter is the last injected key and only a
    // press echoes, so its release is when the console stops moving.
    report.check(
        "the injected editing keys arrive, ending with enter",
        last_key.is_some_and(input::await_armed_key),
    );
    report.check(
        "backspace erases and enter starts a line",
        the_editing_keys_move_the_cursor(),
    );

    // The set 1 path: QEMU honours the translation bit, producing the stream
    // VMware delivers unasked, with a key the harness chose (D-179).
    report.check(
        "the controller accepts translation being turned on",
        input::set_translation(crate::arch::current::i8042::Translation::Enabled),
    );
    report.check(
        "translation updates the decoder from the controller identity",
        input::decoder_matches_identity(),
    );
    // Seeded against the stream on purpose: the round can succeed only by
    // reading the bytes, as VMware forces (D-239).
    input::assume_wrong_numbering();
    // Kept in step with TRANSLATED_MARKER in `xtask/src/qemu.rs`, under the
    // same rule: nothing talks to the keyboard between marker and wait.
    let translated_key = input::arm_key(ps2_keys::Key::B);
    awaiting(
        "selftest: awaiting translated keystroke",
        "  PRESS  b   for the last round",
    );
    // `B`, not `A`, so the first round's stale event cannot satisfy this wait
    // (D-155, D-165).
    report.check(
        "a translated keystroke decodes to the same key",
        translated_key.is_some_and(input::await_armed_key),
    );
    // Ask only after a byte has arrived so this observes the active decoder
    // instead of re-deriving it from the identity that configured it (D-239).
    report.check(
        "the keyboard's numbering and the decoder's are the same",
        input::decoder_reads_what_arrives(),
    );
    // Turned back off, and the question re-opened rather than answered from
    // the written bit; the next decisive byte settles it (D-178, D-239).
    let restored = input::set_translation(crate::arch::current::i8042::Translation::Disabled);
    report.check(
        "the decoder reads the set that arrives, not the bit that was written",
        restored && input::rereads_the_stream(),
    );

    // Last timed check because it advances the console to the bottom.
    report.check(
        "a console scroll does not block interrupts",
        a_scroll_does_not_block_interrupts(),
    );
    info!(
        "selftest: longest console scroll {} ns",
        LONGEST_SCROLL.load(Ordering::Relaxed)
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

    // Read this detector last so every preceding check can trip it (D-143).
    report.check(
        "no lock is taken while a leaf guard is held",
        a_leaf_guard_is_the_last_lock(),
    );

    // Last because it moves the console contents other checks inspect.
    report.check(
        "a scroll moves one visible cursor",
        a_scroll_moves_one_visible_cursor(),
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
        "the unit walks the kernel's root table",
        the_unit_walks_the_kernels_root_table(),
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
    let duplicate_before = trap::rejected_duplicate_eoi_claims();
    let reclaimed_before = trap::reclaimed_abandoned_eoi();
    let first_sent = apic::send_self_interrupt().is_ok();
    report.check(
        "a self-interrupt is delivered",
        first_sent && ping_arrives(before.count + 1),
    );

    let taken = trap::last_ping();
    let second_sent = apic::send_self_interrupt().is_ok();
    report.check(
        "the interrupt is dismissed, so the next one arrives",
        second_sent && ping_arrives(before.count + 2),
    );
    report.check(
        "one delivery mints one completion authority",
        matches!(
            (
                trap::last_ping().count.checked_sub(before.count),
                trap::rejected_duplicate_eoi_claims().checked_sub(duplicate_before),
            ),
            (Some(deliveries), Some(refusals)) if deliveries == refusals
        ),
    );
    report.check(
        "an abandoned completion can be reclaimed for the same delivery",
        matches!(
            (
                trap::last_ping().count.checked_sub(before.count),
                trap::reclaimed_abandoned_eoi().checked_sub(reclaimed_before),
            ),
            (Some(deliveries), Some(reclaims)) if reclaims == usize::from(deliveries >= 2)
        ),
    );
    report.check(
        "the interrupted code was taking interrupts",
        ping_is_consistent(taken),
    );

    let apic_failures =
        apic::initialization_failures_for_test(vector::LOCAL_APIC, || trap::last_ping().count);
    report.check(
        "every late or ignored-write apic refusal stays unavailable and terminal",
        apic_failures.unavailable_and_terminal()
            & apic_failures.ignored_spurious_write_is_terminal(),
    );
    report.check(
        "apic refusal defers a live interrupt until exclusion unwinds",
        apic_failures.delivery_waited_for_unwind(),
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
    debug_exit::report(passed);
}
