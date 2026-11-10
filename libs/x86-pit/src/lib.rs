//! 8254 programmable interval timer encoding, used only as a calibration reference.
//!
//! # Examples
//!
//! ```
//! use x86_pit::{CRYSTAL_HZ, calibrate, elapsed_ticks};
//!
//! // The counter counts down, so a delta is a wrapping subtraction.
//! assert_eq!(elapsed_ticks(1000, 400), 600);
//! assert_eq!(elapsed_ticks(100, 65_436), 200, "through zero mid-window");
//!
//! // Ten cycles per crystal tick is ten times the crystal.
//! assert_eq!(calibrate::measured_hz(200_000, 20_000), 10 * CRYSTAL_HZ as u64);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod calibrate;
pub mod command;

/// The 8254's input clock in hertz, a third of the original 3.579545 MHz colourburst crystal.
pub const CRYSTAL_HZ: u32 = 1_193_182;

/// Bit 6 of a read-back status byte: the written count has not yet reached the counting element.
///
/// A count loads up to 838 ns after its last byte, so a baseline read waits for this to clear.
pub const STATUS_NULL_COUNT: u8 = 1 << 6;

/// Down-counter ticks from `start` to `now`, surviving one wrap; one span is 54.9 ms.
pub const fn elapsed_ticks(start: u16, now: u16) -> u16 {
    start.wrapping_sub(now)
}

#[cfg(test)]
mod tests;
