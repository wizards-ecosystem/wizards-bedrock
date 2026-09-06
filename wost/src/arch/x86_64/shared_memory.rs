//! Publication-word access across an externally writable memory boundary.
//!
//! Aligned dword `mov`s are indivisible, and x86 TSO gives them acquire-load
//! and release-store ordering. Opaque assembly also blocks compiler movement
//! without asking Rust to model the external bytes as an atomic object.

use core::arch::asm;

/// Acquire-load one aligned 32-bit publication word.
///
/// # Safety
///
/// `address` must name four readable, naturally aligned bytes that remain
/// mapped for the instruction. The bytes must not back a live Rust object.
pub(crate) unsafe fn load_acquire_u32(address: usize) -> u32 {
    let value: u32;
    // SAFETY: the caller proves the address is mapped and aligned. A single
    // x86 dword `mov` is indivisible, and omitting memory-effect options keeps
    // this block a compiler barrier without creating a Rust reference.
    unsafe {
        asm!(
            "mov {value:e}, dword ptr [{address}]",
            address = in(reg) address,
            value = out(reg) value,
            options(nostack, preserves_flags),
        );
    }
    value
}

/// Release-store one aligned 32-bit publication word.
///
/// # Safety
///
/// `address` must name four writable, naturally aligned bytes that remain
/// mapped for the instruction. The bytes must not back a live Rust object.
pub(crate) unsafe fn store_release_u32(address: usize, value: u32) {
    // SAFETY: the caller proves the address is mapped and aligned. A single
    // x86 dword `mov` is indivisible, and omitting memory-effect options keeps
    // this block a compiler barrier without creating a Rust reference.
    unsafe {
        asm!(
            "mov dword ptr [{address}], {value:e}",
            address = in(reg) address,
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}
