use super::*;

/// Activation state published to observers of the installed address space.
static ACTIVE_STATE: AtomicU8 = AtomicU8::new(ActivationState::Cold.bits());

/// The root published before activation becomes ready.
static ACTIVE_ROOT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Activation {
    state: &'static AtomicU8,
    root: &'static AtomicU64,
    interrupts: Option<interrupt::InterruptState>,
    committed: bool,
}

impl Activation {
    pub(super) fn begin(
        state: &'static AtomicU8,
        root: &'static AtomicU64,
    ) -> Result<Self, ActivateError> {
        Self::begin_with(state, root, || {})
    }

    pub(super) fn begin_with(
        state: &'static AtomicU8,
        root: &'static AtomicU64,
        after_initializing: impl FnOnce(),
    ) -> Result<Self, ActivateError> {
        let Some(initializing) = ActivationState::Cold.begin() else {
            panic!("cold paging activation cannot begin");
        };
        let interrupts = Some(interrupt::disable());
        match state.compare_exchange(
            ActivationState::Cold.bits(),
            initializing.bits(),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                let activation = Self {
                    state,
                    root,
                    interrupts,
                    committed: false,
                };
                after_initializing();
                Ok(activation)
            }
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

    pub(super) fn commit(self, root: Paddr) {
        self.commit_with(root, || {});
    }

    pub(super) fn commit_with(self, root: Paddr, after_root: impl FnOnce()) {
        let mut this = self;
        this.root.store(root, Ordering::Release);
        after_root();
        let Some(ready) = ActivationState::Initializing.complete() else {
            panic!("paging initialization cannot complete");
        };
        this.committed = true;
        this.state.store(ready.bits(), Ordering::Release);
        if let Some(interrupts) = this.interrupts.take() {
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
            self.state.store(cold.bits(), Ordering::Release);
        }
        if let Some(interrupts) = self.interrupts.take() {
            interrupts.restore();
        }
    }
}

pub(super) fn activation_state(state: &AtomicU8) -> ActivationState {
    let bits = state.load(Ordering::Acquire);
    let Some(state) = ActivationState::from_bits(bits) else {
        panic!("paging activation state is corrupt");
    };
    state
}

/// Switch this CPU to `space` for the kernel's life, after walking it for this code and stack.
///
/// A table that would not survive the switch is refused with the old one still loaded.
pub fn activate(space: &mut AddressSpace) -> Result<(), ActivateError> {
    /// Headroom below the stack pointer that must be mapped too, for the first push.
    const STACK_HEADROOM: u64 = PAGE_SIZE;

    if space.is_reclaimed() {
        #[cfg(feature = "selftest")]
        RECLAIMED_REFUSALS.fetch_add(1, Ordering::Relaxed);
        return Err(ActivateError::Reclaimed);
    }

    space
        .reserve_shared_kernel_topology()
        .map_err(ActivateError::SharedTopology)?;
    let activation = Activation::begin(&ACTIVE_STATE, &ACTIVE_ROOT)?;
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

    // SAFETY: the walks prove this code executable, its stack writable, the direct map whole and
    // unmoved; the device table exists, higher-half topology froze, active roots are not reclaimed.
    unsafe { cpu::set_page_table_base(root) };
    activation.commit(root);

    // `set` never repoints a present entry, and an absent one has no cached translation to flush.
    Ok(())
}

/// Whether the kernel is running on tables it built itself.
pub fn is_active() -> bool {
    activation_state(&ACTIVE_STATE) == ActivationState::Ready
}

/// The root this kernel installed, to compare with [`cpu::page_table_base`] after the switch.
///
/// Only [`is_active`] tells pre-activation apart, because physical zero is a valid root.
pub fn active_root() -> Paddr {
    ACTIVE_ROOT.load(Ordering::Acquire)
}
