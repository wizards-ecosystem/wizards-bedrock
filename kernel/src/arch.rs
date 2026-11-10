//! Architecture-specific kernel code.
//!
//! Callers name [`current`] rather than a concrete architecture.

#[cfg(target_arch = "x86_64")]
pub(crate) mod x86_64;

#[cfg(target_arch = "x86_64")]
pub(crate) use x86_64 as current;
