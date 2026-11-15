use super::*;

/// Activation state published to observers of the installed address space.
static ACTIVE_STATE: AtomicU8 = AtomicU8::new(ActivationState::Cold.bits());

/// The root published before activation becomes ready.
static ACTIVE_ROOT: AtomicU64 = AtomicU64::new(0);

struct Activation {
    interrupts: Option<interrupt::InterruptState>,
    committed: bool,
}

impl Activation {
    fn begin() -> Result<Self, ActivateError> {
        let Some(initializing) = ActivationState::Cold.begin() else {
            panic!("cold paging activation cannot begin");
        };
        let interrupts = Some(interrupt::disable());
        match ACTIVE_STATE.compare_exchange(
            ActivationState::Cold.bits(),
            initializing.bits(),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(Self {
                interrupts,
                committed: false,
            }),
            Err(found) => {
                if let Some(interrupts) = interrupts {
                    interrupts.restore();
                }
                let Some(found) = ActivationState::from_bits(found) else {
                    panic!("paging activation state is corrupt");
                };
                assert!(
                    found != ActivationState::Cold,
                    "a strong activation exchange refused cold state"
                );
                Err(ActivateError::AlreadyActive)
            }
        }
    }

    fn commit(mut self, root: PhysicalAddress) {
        ACTIVE_ROOT.store(root.get(), Ordering::Release);
        let Some(ready) = ActivationState::Initializing.complete() else {
            panic!("paging initialization cannot complete");
        };
        self.committed = true;
        ACTIVE_STATE.store(ready.bits(), Ordering::Release);
        if let Some(interrupts) = self.interrupts.take() {
            interrupts.restore();
        }
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        if !self.committed {
            let Some(cold) = ActivationState::Initializing.abort() else {
                panic!("paging initialization cannot abort");
            };
            ACTIVE_STATE.store(cold.bits(), Ordering::Release);
        }
        if let Some(interrupts) = self.interrupts.take() {
            interrupts.restore();
        }
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
        let entry = EntrySlot::read(&slot);
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

/// Whether the kernel is running on tables it built itself.
pub fn is_active() -> bool {
    let Some(state) = ActivationState::from_bits(ACTIVE_STATE.load(Ordering::Acquire)) else {
        panic!("paging activation state is corrupt");
    };
    state == ActivationState::Ready
}

/// The root this kernel installed, to compare with [`cpu::page_table_base`] after the switch.
///
/// Only [`is_active`] tells pre-activation apart, because physical zero is a valid root.
pub fn active_root() -> PhysicalAddress {
    let Some(root) = PhysicalAddress::new(ACTIVE_ROOT.load(Ordering::Acquire)) else {
        panic!("the published root lies beyond the physical address limit");
    };
    root
}
