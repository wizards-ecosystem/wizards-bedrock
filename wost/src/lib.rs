//! Wizard OS Standard — the privileged kernel framework.
//!
//! `wost` is the only crate in the kernel permitted to write `unsafe`. Every
//! other kernel crate is safe Rust built on the abstractions exported here.
//! See `docs/002-architecture.md` pillar 5 and `docs/005-code-conventions.md`.

// This is the workspace's sole unsafe exception (D-67). `allow` is deliberate:
// `expect` would make removing the final unsafe operation a build failure.
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
