//! Width conversions for hardware register values.
//!
//! A 32-bit register read through a wider access arrives with reserved high
//! bits. Readers ignore them here, in one place, before any comparison or
//! decode, and never refuse a value because of them. Writes of a 64-bit
//! operand to two 32-bit halves split it here too.
//!
//! # Examples
//!
//! ```
//! use register_bits::{low_u32, u16_halves, u32_halves};
//!
//! assert_eq!(low_u32(0xFFFF_FFFF_0000_0001), 1);
//! assert_eq!(u32_halves(0x1234_5678_9ABC_DEF0), (0x9ABC_DEF0, 0x1234_5678));
//! assert_eq!(u16_halves(0x1234_8086), (0x8086, 0x1234));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

const _: () = assert!(
    usize::BITS == u64::BITS,
    "a 64-bit register offset is a usize only on 64-bit targets"
);

/// The 32-bit register held in the low half of a 64-bit read, with the
/// reserved high half ignored.
pub const fn low_u32(value: u64) -> u32 {
    value as u32
}

/// The low and high halves of a 64-bit operand, low first.
pub const fn u32_halves(value: u64) -> (u32, u32) {
    (value as u32, (value >> 32) as u32)
}

/// The low and high halves of a 32-bit register, low first.
pub const fn u16_halves(value: u32) -> (u16, u16) {
    (value as u16, (value >> 16) as u16)
}

/// A 64-bit register offset as an index into a register window.
pub const fn usize_from_u64(value: u64) -> usize {
    value as usize
}

#[cfg(test)]
mod tests;
