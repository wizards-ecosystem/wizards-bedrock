use super::*;

impl NvmeBusMasterAuthority<'_> {
    /// The live register values carried by this authority.
    #[cfg(feature = "selftest")]
    pub fn observation_for_test(&self) -> (u32, u32) {
        self._observed.bits()
    }
}

/// Whether any offset of `at` is claimed by a live function authority.
#[cfg(feature = "selftest")]
pub fn is_claimed(at: Address) -> bool {
    CONFIGURATION.lock_leaf().functions.is_claimed(at)
}

/// Function authorities retained at this instant.
#[cfg(feature = "selftest")]
pub fn claims() -> usize {
    CONFIGURATION.lock_leaf().functions.len()
}
