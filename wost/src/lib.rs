#![expect(unsafe_code, reason = "the one crate that may write unsafe")]
#![no_std]

extern crate alloc;

pub mod arch;
pub mod boot;
mod cell;
pub mod mm;
pub mod mmio;
pub mod panic;
pub mod sync;
