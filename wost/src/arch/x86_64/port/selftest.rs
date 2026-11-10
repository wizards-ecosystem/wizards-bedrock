use super::*;

#[cfg(feature = "selftest")]
pub(super) static REFUSE_SPAN_RELEASE: AtomicBool = AtomicBool::new(false);

/// Refuse the next dynamic span release before the registry changes.
#[cfg(feature = "selftest")]
pub fn refuse_span_release_for_test() {
    REFUSE_SPAN_RELEASE.store(true, Ordering::Release);
}

/// Whether `len` ports from `port` are already claimed, without minting a [`Port`] over them.
#[cfg(feature = "selftest")]
pub fn is_claimed(port: u16, len: usize) -> bool {
    !CLAIMED.lock_leaf().is_available(port as usize, len)
}
