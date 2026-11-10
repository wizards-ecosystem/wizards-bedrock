use super::*;

#[cfg(feature = "selftest")]
static RUST_USER_ENTRIES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
static INVALID_RUST_USER_ENTRIES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
static COMPLETE_PAIR_SWITCHES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
static DISTINCT_PAIR_SWITCHES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
static HAVE_LAST_PAIR: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static LAST_FULL_ROOT: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
static LAST_STRIPPED_ROOT: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
pub(super) fn record_complete_pair_switch(roots: AddressSpaceRoots) {
    let Some(stripped) = roots.stripped_root() else {
        return;
    };
    let full = roots.full_root();
    COMPLETE_PAIR_SWITCHES.fetch_add(1, Ordering::Relaxed);
    let had_last = HAVE_LAST_PAIR.swap(true, Ordering::AcqRel);
    let previous_full = LAST_FULL_ROOT.swap(full, Ordering::AcqRel);
    let previous_stripped = LAST_STRIPPED_ROOT.swap(stripped, Ordering::AcqRel);
    if !had_last || previous_full != full || previous_stripped != stripped {
        DISTINCT_PAIR_SWITCHES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Record the root on which a user-origin frame first reaches Rust.
#[cfg(feature = "selftest")]
pub(crate) fn record_rust_entry(code_selector: u64) {
    if code_selector & 0b11 != 3 {
        return;
    }
    RUST_USER_ENTRIES.fetch_add(1, Ordering::Relaxed);
    let full = WOST_KPTI_ROUTE.full_root.load(Ordering::Acquire);
    if WOST_KPTI_ROUTE.state.load(Ordering::Acquire) != ROUTE_PUBLISHED
        || cpu::page_table_base() != full
    {
        INVALID_RUST_USER_ENTRIES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Whether every stripped-root arrival reached Rust only after the full load.
#[cfg(feature = "selftest")]
pub(crate) fn every_user_entry_reached_rust_on_full_root() -> bool {
    let stripped = WOST_KPTI_ROUTE.stripped_entries.load(Ordering::Acquire);
    stripped > 0
        && RUST_USER_ENTRIES.load(Ordering::Acquire) == stripped
        && INVALID_RUST_USER_ENTRIES.load(Ordering::Acquire) == 0
}

/// Whether every prepared user return was next observed on the stripped root.
#[cfg(feature = "selftest")]
pub(crate) fn every_user_return_installed_the_stripped_root() -> bool {
    let prepared = WOST_KPTI_ROUTE.prepared_returns.load(Ordering::Acquire);
    prepared > 0 && WOST_KPTI_ROUTE.stripped_entries.load(Ordering::Acquire) == prepared
}

/// Whether task switches installed complete routes for at least two pairs.
#[cfg(feature = "selftest")]
pub(crate) fn context_switch_moved_complete_root_pairs() -> bool {
    COMPLETE_PAIR_SWITCHES.load(Ordering::Acquire) >= 2
        && DISTINCT_PAIR_SWITCHES.load(Ordering::Acquire) >= 2
}

/// Whether both guarded enclave stacks contain no retained transition bytes.
#[cfg(feature = "selftest")]
pub(crate) fn entry_stacks_are_guarded_and_scrubbed() -> bool {
    if WOST_KPTI_ROUTE.state.load(Ordering::Acquire) != ROUTE_EMPTY {
        return false;
    }
    let Some(enclave) = entry_enclave() else {
        return false;
    };
    let guards_absent = enclave.guards().into_iter().all(|guard| {
        (guard.start..guard.end)
            .step_by(PAGE_BYTES as usize)
            .all(|address| super::paging::translate(address).is_none())
    });
    if !guards_absent || !segment::enclave_ist_stack_is_zeroed_for_test() {
        return false;
    }
    let bytes = TRANSITION_STACK.as_mut_ptr().cast::<u8>();
    (0..STACK_BYTES).all(|offset| {
        // SAFETY: the route is empty, so hardware cannot be using the
        // transition stack; the linker-owned allocation spans STACK_BYTES.
        let byte = unsafe { bytes.add(offset) };
        // SAFETY: `byte` is inside that same inactive allocation.
        unsafe { byte.read_volatile() == 0 }
    })
}
