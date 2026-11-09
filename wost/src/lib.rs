//! Bedrock's privileged kernel framework.
//!
//! `wost` is the only crate in the kernel permitted to write `unsafe`. Every
//! other kernel crate is safe Rust built on the abstractions exported here.
//! See `docs/002-architecture.md` pillar 5 and `docs/005-code-conventions.md`.

#![allow(
    unsafe_code,
    reason = "the one unsafe crate; expect fails once its last unsafe goes"
)]
#![deny(clippy::expect_used, clippy::unwrap_used)]
#![no_std]

extern crate alloc;

pub mod arch;
pub mod boot;
mod cell;
pub mod mm;
pub mod mmio;
pub mod panic;
pub mod sync;
