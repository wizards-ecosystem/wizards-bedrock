use register_bits::u16_halves;

// Below this is the discrete 82489DX.
const FIRST_INTEGRATED: u8 = 0x10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Version(u32);

impl Version {
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn number(self) -> u8 {
        u16_halves(self.0).0 as u8
    }

    pub const fn maximum_lvt_entry(self) -> u8 {
        u16_halves(self.0).1 as u8
    }

    pub const fn is_integrated(self) -> bool {
        self.number() >= FIRST_INTEGRATED
    }
}
