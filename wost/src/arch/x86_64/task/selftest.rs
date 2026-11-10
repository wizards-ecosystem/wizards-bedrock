use super::*;

#[cfg(feature = "selftest")]
pub(super) static REFUSE_NEXT_RETIREMENT: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
pub(super) static REFUSE_RELEASES: AtomicUsize = AtomicUsize::new(0);

/// Run the production arena validation against the active root.
#[cfg(feature = "selftest")]
pub fn validate_arena_for_test(arena: &StackArena) -> Result<(), TaskError> {
    validate_arena(arena).map(|_| ())
}

/// Substitute one noncurrent pinned stack leaf during its hardware-use check.
#[cfg(feature = "selftest")]
pub fn pinned_stack_rejects_lower_leaf_substitution_for_test(slot: usize) -> Option<bool> {
    if slot == current() {
        return None;
    }
    let table = TABLE.lock_leaf();
    let index = table.slots.get(slot)?.stack_index?;
    table
        .stacks
        .as_ref()?
        .rejects_lower_leaf_substitution_for_test(index)
}

/// Make the next prepared-slot retirement refuse without changing the slot.
#[cfg(feature = "selftest")]
pub fn refuse_next_retirement_for_test() {
    REFUSE_NEXT_RETIREMENT.store(1, Ordering::Release);
}

/// Clear an unused retirement refusal after a test setup failed early.
#[cfg(feature = "selftest")]
pub fn restore_retirement_for_test() {
    REFUSE_NEXT_RETIREMENT.store(0, Ordering::Release);
}

/// Whether a task-table slot is still live.
#[cfg(feature = "selftest")]
pub fn slot_is_live_for_test(slot: usize) -> bool {
    let saved = interrupt::disable();
    let live = TABLE
        .lock_leaf()
        .slots
        .get(slot)
        .is_some_and(|slot| slot.state == State::Alive);
    saved.restore();
    live
}

/// Keep address-space release from consuming its `Bound` token.
#[cfg(feature = "selftest")]
pub fn refuse_releases_for_test() {
    REFUSE_RELEASES.store(1, Ordering::Release);
}

/// Clear an unused address-space release refusal.
#[cfg(feature = "selftest")]
pub fn restore_release_for_test() {
    REFUSE_RELEASES.store(0, Ordering::Release);
}

/// Exercise the final door while its interrupt admission is absent.
#[cfg(feature = "selftest")]
pub fn final_interrupt_admission_refusal_for_test() -> bool {
    if !interrupt::are_enabled() {
        return false;
    }
    let outer = interrupt::disable();
    let refused = enter_user(0, 0) == UserError::InterruptsNotAdmitted;
    let stayed_disabled = !interrupt::are_enabled();
    outer.restore();
    refused && stayed_disabled && interrupt::are_enabled()
}

#[cfg(feature = "selftest")]
pub(super) static FORCED_USER_BOUNDARY_REFUSAL: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
fn forced_boundary_refuses(value: usize, expected: UserError) -> bool {
    let previous = FORCED_USER_BOUNDARY_REFUSAL.swap(value, Ordering::AcqRel);
    let found = user_entry_preflight(0, 0) == Err(expected);
    FORCED_USER_BOUNDARY_REFUSAL.store(previous, Ordering::Release);
    found
}

/// Exercise the exact final-door NX refusal.
#[cfg(feature = "selftest")]
pub fn final_no_execute_refusal_for_test() -> bool {
    forced_boundary_refuses(1, UserError::NoExecuteProtection)
}

/// Exercise the exact final-door extended-state refusal.
#[cfg(feature = "selftest")]
pub fn final_extended_state_refusal_for_test() -> bool {
    xstate::without_user_state_isolation_for_test(|| {
        user_entry_preflight(0, 0) == Err(UserError::NoExtendedState)
    })
}

/// Exercise the exact final-door shared-mapping refusal.
#[cfg(feature = "selftest")]
pub fn final_shared_mapping_refusal_for_test() -> bool {
    forced_boundary_refuses(2, UserError::NoSharedKernelIsolation)
}
