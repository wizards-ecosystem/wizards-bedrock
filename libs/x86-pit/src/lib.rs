#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod calibrate;
pub mod command;

pub const CRYSTAL_HZ: u32 = 1_193_182;

// Set until the written count reaches the counting element, up to 838 ns after its last byte.
pub const STATUS_NULL_COUNT: u8 = 1 << 6;

// A window over one 54.9 ms span reads as a shorter one, so the caller must keep it shorter.
pub const fn elapsed_ticks(start: u16, now: u16) -> u16 {
    start.wrapping_sub(now)
}
