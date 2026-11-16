use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use x86_paging::{ENTRIES, Flags, PAGE_SIZE};

use super::super::{cpu, interrupt};
use super::{
    ActivateError, AddressSpace, direct_map_gap, entry_slot, handed_out_gap, kernel_image_gap,
    translate_in,
};
use crate::mm::PhysicalAddress;

const COLD: u8 = 0;
const INITIALIZING: u8 = 1;
const READY: u8 = 2;

static ACTIVE_STATE: AtomicU8 = AtomicU8::new(COLD);

static ACTIVE_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct Activation {
    interrupts: interrupt::InterruptState,
    committed: bool,
}

impl Activation {
    fn begin() -> Result<Self, ActivateError> {
        let interrupts = interrupt::disable();
        if ACTIVE_STATE
            .compare_exchange(COLD, INITIALIZING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            interrupts.restore();
            return Err(ActivateError::AlreadyActive);
        }
        Ok(Self {
            interrupts,
            committed: false,
        })
    }

    fn commit(mut self, root: PhysicalAddress) {
        ACTIVE_ROOT.store(root.get(), Ordering::Release);
        self.committed = true;
        ACTIVE_STATE.store(READY, Ordering::Release);
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        if !self.committed {
            ACTIVE_STATE.store(COLD, Ordering::Release);
        }
        self.interrupts.restore_in_place();
    }
}

pub fn activate(space: &mut AddressSpace) -> Result<(), ActivateError> {
    // Mapped too, for the first push after the switch.
    const STACK_HEADROOM: u64 = PAGE_SIZE;

    // Every data leaf carries bit 63, which is reserved until `EFER.NXE` is set.
    if !cpu::Protections::enabled().nx {
        return Err(ActivateError::NoExecuteDisabled);
    }
    if space.is_reclaimed() {
        return Err(ActivateError::Reclaimed);
    }

    space
        .reserve_shared_kernel_topology()
        .map_err(ActivateError::SharedTopology)?;
    // User roots copy these entries, so a `USER` one would share ring-3 pages across processes.
    for index in ENTRIES / 2..ENTRIES {
        let slot = entry_slot(space.root(), index).map_err(ActivateError::SharedTopology)?;
        let entry = slot.read();
        if entry.is_present() && entry.flags().contains(Flags::USER) {
            return Err(ActivateError::UserKernelEntry(index));
        }
    }
    let activation = Activation::begin()?;
    let root = space.root();
    let current_root = cpu::page_table_base();
    let code = activate as *const () as u64;
    match translate_in(root, code) {
        None => return Err(ActivateError::CodeUnmapped(code)),
        Some((_, flags)) if flags.contains(Flags::NO_EXECUTE) => {
            return Err(ActivateError::CodeNotExecutable(code));
        }
        Some((physical, _)) => {
            if translate_in(current_root, code).map(|current| current.0) != Some(physical) {
                return Err(ActivateError::MappingChanged(code));
            }
        }
    }

    let stack = cpu::stack_pointer();
    for probe in [stack, stack.saturating_sub(STACK_HEADROOM)] {
        match translate_in(root, probe) {
            None => return Err(ActivateError::StackUnmapped(probe)),
            Some((_, flags)) if !flags.contains(Flags::WRITABLE) => {
                return Err(ActivateError::StackReadOnly(probe));
            }
            Some((_, flags))
                if flags.contains(Flags::USER)
                    || !flags.contains(Flags::NO_EXECUTE)
                    || flags.contains(Flags::NO_CACHE)
                    || flags.contains(Flags::WRITE_THROUGH) =>
            {
                return Err(ActivateError::StackUnsafe(probe));
            }
            Some((physical, _)) => {
                if translate_in(current_root, probe).map(|current| current.0) != Some(physical) {
                    return Err(ActivateError::MappingChanged(probe));
                }
            }
        }
    }

    if let Some(gap) = kernel_image_gap(root, current_root) {
        return Err(ActivateError::ImageChanged(gap));
    }

    if let Some(gap) = direct_map_gap(root, current_root) {
        return Err(ActivateError::DirectMapGap(gap));
    }

    if let Some(gap) = handed_out_gap(root) {
        return Err(ActivateError::HandoffUnmapped(gap));
    }

    // SAFETY: NXE is set, so no leaf holds a reserved bit; the walks prove this code executable,
    // its stack writable, the direct map whole and unmoved with every framebuffer and firmware
    // byte; the device table exists, higher-half topology froze, active roots are not reclaimed.
    unsafe { cpu::set_page_table_base(root) };
    activation.commit(root);
    Ok(())
}

pub(crate) fn is_active() -> bool {
    ACTIVE_STATE.load(Ordering::Acquire) == READY
}

// Only `is_active` tells pre-activation apart, because physical zero is a valid root.
pub(crate) fn active_root() -> PhysicalAddress {
    let Some(root) = PhysicalAddress::new(ACTIVE_ROOT.load(Ordering::Acquire)) else {
        panic!("the published root lies beyond the physical address limit");
    };
    root
}
