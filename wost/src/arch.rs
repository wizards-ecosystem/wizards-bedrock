//! Architecture-specific primitives.
//!
//! Consumers name [`current`] rather than a concrete architecture, so porting
//! means adding a sibling module and one `cfg` arm.

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use x86_64 as current;
