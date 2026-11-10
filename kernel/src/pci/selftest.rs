use super::*;

/// Whether a bus named by many bridges still leaves room for a later one.
#[cfg(feature = "selftest")]
pub(crate) fn repeated_secondaries_do_not_hide_a_later_bus_for_test() -> bool {
    let mut worklist = Worklist::new();
    worklist.push(0);
    for _ in 0..255 {
        worklist.push(1);
    }
    worklist.push(5)
        && worklist.pop() == Some(0)
        && worklist.pop() == Some(1)
        && worklist.pop() == Some(5)
        && worklist.pop().is_none()
}

/// Make the first real DMA admission exercise the post-disable rollback path.
#[cfg(feature = "selftest")]
pub(super) static FORCED_BUS_MASTER_REFUSALS: AtomicUsize = AtomicUsize::new(1);

/// Exercise cleanup after the function and its BAR shape have been claimed.
#[cfg(feature = "selftest")]
pub(super) static REFUSE_CLEAN_PREPARATION: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static OBSERVED_BUS_MASTER_REFUSAL: AtomicBool = AtomicBool::new(false);

/// Whether the deterministic post-disable DMA-admission refusal was consumed.
#[cfg(feature = "selftest")]
pub(crate) fn bus_master_refusal_was_observed() -> bool {
    OBSERVED_BUS_MASTER_REFUSAL.load(Ordering::Acquire)
}

/// Refuse the next clean preparation after its BAR probe has settled.
#[cfg(feature = "selftest")]
pub(crate) fn refuse_clean_preparation_for_test() {
    assert!(
        !REFUSE_CLEAN_PREPARATION.swap(true, Ordering::Relaxed),
        "a clean PCI preparation refusal was already armed"
    );
}

/// Whether the function at `at` is claimed.
#[cfg(feature = "selftest")]
pub(crate) fn is_claimed(at: Address) -> bool {
    pci::is_claimed(at)
}

/// How many functions are claimed.
#[cfg(feature = "selftest")]
pub(crate) fn claims() -> usize {
    pci::claims()
}
