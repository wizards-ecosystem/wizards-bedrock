#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Lifecycle {
    Cold,
    Initializing,
    Ready,
    Poisoned,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interrupts {
    Excluded,
    Enabled,
}

impl Lifecycle {
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Cold),
            1 => Some(Self::Initializing),
            2 => Some(Self::Ready),
            3 => Some(Self::Poisoned),
            _ => None,
        }
    }

    pub const fn bits(self) -> u8 {
        self as u8
    }

    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}
