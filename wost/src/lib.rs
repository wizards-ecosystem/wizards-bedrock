//! Bedrock's privileged kernel framework.
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

/// `usize` from a `u64` length or register offset.
///
/// # Panics
///
/// Panics when `value` does not fit in `usize`. On `x86_64` the widths match,
/// so a displacement in a `u64` register constant cannot reach this.
pub(crate) const fn usize_from_u64(value: u64) -> usize {
    assert!(
        usize::BITS >= 64 || value <= 0xFFFF_FFFF,
        "u64 does not fit in usize"
    );
    // The range check is the conversion. `as` remains because `TryFrom` is not
    // a stable const trait, and on x86_64 the check is dead.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rejected above when usize is narrower than u64"
    )]
    {
        value as usize
    }
}

/// A 32-bit register value carried zero-extended in a `u64`.
///
/// # Panics
///
/// Panics when the upper half is nonzero. That is a broken register read, not
/// a value this kernel stores by discarding bits.
pub(crate) const fn u32_from_u64(value: u64) -> u32 {
    assert!(
        value <= 0xFFFF_FFFF,
        "u64 register value does not fit in u32"
    );
    #[expect(clippy::cast_possible_truncation, reason = "value is at most u32::MAX")]
    {
        value as u32
    }
}

/// `u16` from a `usize` that a 16-bit hardware field must hold whole.
///
/// # Panics
///
/// Panics when `value` does not fit in `u16`.
pub(crate) const fn u16_from_usize(value: usize) -> u16 {
    assert!(value <= 0xFFFF, "usize does not fit in u16");
    #[expect(clippy::cast_possible_truncation, reason = "value is at most u16::MAX")]
    {
        value as u16
    }
}

/// Low and high halves of a 64-bit register operand (`eax`, then `edx`).
///
/// The low half is the low 32 bits on purpose. The high half is what remains
/// after the shift, which always fits.
pub(crate) const fn u32_halves(value: u64) -> (u32, u32) {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the low half of a 64-bit register operand is its low 32 bits"
    )]
    let low = value as u32;
    // Shifting by 32 leaves at most 32 bits, so this is not a narrowing clippy
    // still treats as a truncation.
    let high = (value >> 32) as u32;
    (low, high)
}

pub mod arch;
pub mod boot;
mod cell;
pub mod mm;
pub mod mmio;
pub mod panic;
pub mod sync;
