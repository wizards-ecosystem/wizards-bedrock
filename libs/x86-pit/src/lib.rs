//! 8254 programmable interval timer encoding.
//!
//! Bit positions follow the Intel 8254 datasheet's control-word figure; the kernel uses
//! the chip only as a calibration reference, never as an interrupt source.
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
#![warn(clippy::cast_possible_truncation)]
#![warn(clippy::exhaustive_enums)]

pub mod calibrate;
pub mod command;

/// The 8254's input clock in hertz — 1.193182 MHz on every PC compatible,
/// a third of the original machine's 3.579545 MHz colourburst crystal.
pub const CRYSTAL_HZ: u32 = 1_193_182;

/// Bit 6 of a read-back status byte: the written count has not yet reached
/// the counting element.
pub const STATUS_NULL_COUNT: u8 = 1 << 6;

/// Timer ticks between two readings of the down-counter, surviving one wrap.
///
/// The counter counts down, so the distance from `start` to `now` is their difference
/// modulo the counter's width — which is exactly what wrapping subtraction on `u16`
/// computes.
pub const fn elapsed_ticks(start: u16, now: u16) -> u16 {
    start.wrapping_sub(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_delta_is_a_subtraction() {
        assert_eq!(elapsed_ticks(1000, 400), 600);
    }

    /// The counter runs through zero mid-window: 100 ticks to the wrap, then
    /// another 100 down from the top.
    #[test]
    fn a_delta_survives_the_wrap() {
        assert_eq!(elapsed_ticks(100, 65_436), 200);
    }

    #[test]
    fn an_unmoved_counter_is_zero_ticks() {
        assert_eq!(elapsed_ticks(5, 5), 0);
    }

    #[test]
    fn the_longest_measurable_window_is_one_count_short_of_the_span() {
        assert_eq!(elapsed_ticks(0, 1), u16::MAX);
    }

    /// The status byte's null-count flag is bit 6.
    #[test]
    fn the_null_count_flag_is_bit_six() {
        // 8254 datasheet, read-back status: bit 7 output, bit 6 null count.
        assert_eq!(STATUS_NULL_COUNT, 0b0100_0000);
    }
}
