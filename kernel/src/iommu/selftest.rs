use super::*;

/// Last complete live requester inventory installed by a successful retarget.
#[cfg(feature = "selftest")]
pub(super) static INSTALLED_REQUESTERS: AtomicUsize = AtomicUsize::new(0);

/// Successful candidate retargets, including the candidate replaced by fallback.
#[cfg(feature = "selftest")]
pub(super) static SUCCESSFUL_RETARGETS: AtomicUsize = AtomicUsize::new(0);

/// The exact lookup policy, for the machine check that feeds it regions.
#[cfg(feature = "selftest")]
pub(crate) fn reserved_regions_are_supported_for_test(reserved: &[(u64, u64)]) -> bool {
    reserved_regions_are_supported(reserved.iter().copied())
}

/// Whether the live PCI interrupt boundary retains every deny-all property.
#[cfg(feature = "selftest")]
pub(crate) fn interrupt_boundary_is_deny_all_for_test() -> bool {
    if matches!(pci::topology_presence(), pci::TopologyPresence::Unsealed) {
        return false;
    }
    INSTALLED.lock().as_ref().is_some_and(|unit| {
        let facts = unit.facts();
        unit.state == InstalledState::Ready
            && facts.interrupt_remapping
            && facts.queued_invalidation
            && unit.boundary_active()
    })
}

/// Whether the default machine retargeted one continuous owner twice while it
/// still holds one fence for every non-selected requester.
#[cfg(feature = "selftest")]
pub(crate) fn requester_fences_cover_shared_unit_for_test() -> bool {
    if DMAR.lock_leaf().is_none() {
        return true;
    }
    let requesters = INSTALLED_REQUESTERS.load(Ordering::Acquire);
    let installed = INSTALLED.lock();
    requesters > 1
        && SUCCESSFUL_RETARGETS.load(Ordering::Acquire) >= 2
        && installed.as_ref().is_some_and(|unit| {
            unit.state == InstalledState::Ready
                && unit.fences.count + 1 == requesters
                && unit.boundary_active()
        })
        && pci::claims() == requesters
}
