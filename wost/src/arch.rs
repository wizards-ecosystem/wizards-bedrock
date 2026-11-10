//! Architecture-specific primitives; consumers name [`current`], so a port adds one `cfg` arm.

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use x86_64 as current;
