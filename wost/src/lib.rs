//! Wizard OS Standard — the privileged kernel framework.
//!
//! `wost` is the only crate in the kernel permitted to write `unsafe`. Every
//! other kernel crate is safe Rust built on the abstractions exported here.
//! See `docs/002-architecture.md` pillar 5 and `docs/005-code-conventions.md`.

// `wost` is the exception the workspace default exists to make visible: the
// deny lives in `Cargo.toml`, and this is the one place it is lifted (D-67).
//
// `allow` and not `expect`, deliberately, where D-131 converted every other
// suppression in this crate. An `expect` here would assert that `wost`
// *contains* unsafe; the rule is that `wost` is the only crate that **may**.
// A refactor that emptied it would be the framekernel's best day and would
// fail this build.
#![allow(unsafe_code)]
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
