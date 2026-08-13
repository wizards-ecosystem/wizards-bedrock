//! x86_64 device drivers.

#[cfg(feature = "selftest")]
pub(crate) mod debug_exit;
pub(crate) mod uart;
