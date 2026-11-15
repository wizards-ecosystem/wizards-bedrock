#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod exception;
pub mod gdt;
pub mod idt;
pub mod return_state;
pub mod tss;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selector(u16);

impl Selector {
    pub const fn new(index: u16, privilege: u16) -> Self {
        assert!(index < 0x2000, "descriptor index does not fit a selector");
        assert!(privilege < 4, "privilege level is two bits");
        Self(index << 3 | privilege)
    }

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn index(self) -> u16 {
        self.0 >> 3
    }
}
