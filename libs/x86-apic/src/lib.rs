#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

pub mod base;
pub mod in_service;
pub mod io;
pub mod ipi;
pub mod lifecycle;
pub mod local_vector;
pub mod pending;
pub mod register;
pub mod spurious;
pub mod timer;
pub mod vector;
pub mod version;

const REGISTER_BYTES: usize = 4096;

const FIRST_MSR: u32 = 0x800;

const REGISTER_STRIDE: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Register(usize);

impl Register {
    pub const fn new(offset: usize) -> Self {
        // Subtraction, because `offset + 4` wraps near `usize::MAX`.
        assert!(
            offset <= REGISTER_BYTES - size_of::<u32>(),
            "register lies outside the local APIC page"
        );
        assert!(
            offset.is_multiple_of(REGISTER_STRIDE),
            "local APIC registers are 16-byte aligned"
        );
        Self(offset)
    }

    // Each sixteen-byte slot of the page is one consecutive x2APIC MSR.
    pub const fn msr(self) -> u32 {
        FIRST_MSR + (self.0 / REGISTER_STRIDE) as u32
    }
}
