use crate::vector::InterruptVector;

const ENABLE: u32 = 1 << 8;

// The earliest APICs hardwire a spurious vector's low four bits to one.
const HARDWIRED_LOW_BITS: u8 = 0x0F;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spurious(u32);

impl Spurious {
    pub const fn enabled(vector: InterruptVector) -> Self {
        let vector = vector.get();
        assert!(
            vector & HARDWIRED_LOW_BITS == HARDWIRED_LOW_BITS,
            "a spurious vector must have its low four bits set"
        );
        Self(vector as u32 | ENABLE)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn is_enabled(self) -> bool {
        self.0 & ENABLE != 0
    }
}
