//! Wizard OS Standard — the privileged kernel framework.
//!
//! `wost` is the only crate in the kernel permitted to write `unsafe`. Every
//! other kernel crate is safe Rust built on the abstractions exported here.
//! See `docs/002-architecture.md` pillar 5 and `docs/005-code-conventions.md`.

#![no_std]

pub mod arch;
pub mod boot;
mod cell;
pub mod mm;
pub mod mmio;
pub mod panic;
pub mod sync;
