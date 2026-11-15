#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum State {
    Cold = 0,
    Initializing = 1,
    Ready = 2,
}

impl State {
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Cold),
            1 => Some(Self::Initializing),
            2 => Some(Self::Ready),
            _ => None,
        }
    }

    pub const fn bits(self) -> u8 {
        self as u8
    }

    pub const fn begin(self) -> Option<Self> {
        match self {
            Self::Cold => Some(Self::Initializing),
            Self::Initializing | Self::Ready => None,
        }
    }

    pub const fn complete(self) -> Option<Self> {
        match self {
            Self::Initializing => Some(Self::Ready),
            Self::Cold | Self::Ready => None,
        }
    }

    pub const fn abort(self) -> Option<Self> {
        match self {
            Self::Initializing => Some(Self::Cold),
            Self::Cold | Self::Ready => None,
        }
    }
}
