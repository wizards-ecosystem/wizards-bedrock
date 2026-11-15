//! Bedrock's privileged kernel framework, the only kernel crate permitted to write `unsafe`.

#![allow(
    unsafe_code,
    reason = "the one unsafe crate; expect fails once its last unsafe goes"
)]
#![no_std]

extern crate alloc;

pub mod arch;
pub mod boot;
mod cell;
pub mod mm;
pub mod mmio;
pub mod panic;
pub mod sync;
