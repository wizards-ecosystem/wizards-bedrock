#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

const _: () = assert!(
    usize::BITS == u64::BITS,
    "a 64-bit register offset is a usize only on 64-bit targets"
);

pub const fn low_u32(value: u64) -> u32 {
    value as u32
}

pub const fn u32_halves(value: u64) -> (u32, u32) {
    (value as u32, (value >> 32) as u32)
}

pub const fn u16_halves(value: u32) -> (u16, u16) {
    (value as u16, (value >> 16) as u16)
}

pub const fn usize_from_u64(value: u64) -> usize {
    value as usize
}
