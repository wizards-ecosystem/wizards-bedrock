use wost::arch::current::cpu;

use wost::arch::current::{port, xstate};

use crate::arch::current::uart;

use crate::input;
use crate::iommu;

pub(super) fn default_deny_interrupt_remapping_guards_pci() -> bool {
    iommu::interrupt_boundary_is_deny_all_for_test()
        && iommu::interrupt_boundary_active()
        && !input::is_interrupt_driven()
}

pub(super) fn ports_are_exclusive() -> bool {
    port::claim_block::<8>(uart::COM1).is_none() && port::claim::<u8>(uart::COM1).is_none()
}

pub(super) fn a_save_stays_inside_its_area() -> bool {
    if !xstate::is_enabled() {
        // The bounds check has no instruction to exercise; user launch refuses.
        return xstate::bytes() == 0;
    }
    // The form is whatever the processor supports; what must hold on every
    // processor is that the area was sized for the form in use.
    xstate::save_stays_inside_its_area()
}

pub(super) fn the_fast_syscall_doors_are_shut_by_this_kernel() -> bool {
    cpu::drive_fast_syscall_shutdown_for_test()
}

pub(super) fn a_claim_covers_the_width_it_drives() -> bool {
    const BELOW_COM1: u16 = uart::COM1 - 2;

    port::claim::<u32>(BELOW_COM1).is_none()
        && port::claim::<u8>(BELOW_COM1).is_some()
        && port::claim::<u8>(BELOW_COM1).is_none()
}

/// Every input line on both legacy controllers, disconnected.
pub(super) const MASKED: u8 = 0xFF;
