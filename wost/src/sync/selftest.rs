use core::sync::atomic::{AtomicBool, Ordering};

/// A machine-test fault-injection switch, unarmed until a check holds it for one call.
#[cfg(feature = "selftest")]
#[derive(Debug)]
pub struct Injection(AtomicBool);

#[cfg(feature = "selftest")]
impl Injection {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// Whether a running check holds this switch armed.
    pub fn armed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Run `check` with the switch set to `value`, then restore the setting it replaced.
    pub fn with<T>(&self, value: bool, check: impl FnOnce() -> T) -> T {
        let previous = self.0.swap(value, Ordering::AcqRel);
        let outcome = check();
        self.0.store(previous, Ordering::Release);
        outcome
    }
}

#[cfg(feature = "selftest")]
impl Default for Injection {
    fn default() -> Self {
        Self::new()
    }
}
