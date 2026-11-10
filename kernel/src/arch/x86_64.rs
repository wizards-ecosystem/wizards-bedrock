//! x86_64 device drivers.

#[cfg(feature = "selftest")]
pub(crate) mod debug_exit;
pub(crate) mod i8042;
pub(crate) mod ioapic;
pub(crate) mod pit;
pub(crate) mod trap;
pub(crate) mod uart;
pub(crate) mod vector;
