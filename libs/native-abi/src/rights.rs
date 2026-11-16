#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rights(u32);

impl Rights {
    pub const NONE: Self = Self(0);

    pub const DUPLICATE: Self = Self(1 << 0);

    pub const TRANSFER: Self = Self(1 << 1);

    pub const WAIT: Self = Self(1 << 2);

    pub const INSPECT: Self = Self(1 << 3);

    pub const UNIVERSAL: Self = Self(0b1111);

    pub const READ: Self = Self(1 << 8);

    pub const WRITE: Self = Self(1 << 9);

    // Never held with `WRITE` in a mapping.
    pub const EXECUTE: Self = Self(1 << 10);

    pub const MAP: Self = Self(1 << 11);

    pub const TRAVERSE: Self = Self(1 << 12);

    pub const ENUMERATE: Self = Self(1 << 13);

    pub const MODIFY: Self = Self(1 << 14);

    // An undefined bit refuses the whole word: masking would grant quietly.
    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !Self::ALL.0 != 0 {
            return None;
        }
        Some(Self(bits))
    }

    pub const ALL: Self = Self(
        Self::DUPLICATE.0
            | Self::TRANSFER.0
            | Self::WAIT.0
            | Self::INSPECT.0
            | Self::READ.0
            | Self::WRITE.0
            | Self::EXECUTE.0
            | Self::MAP.0
            | Self::TRAVERSE.0
            | Self::ENUMERATE.0
            | Self::MODIFY.0,
    );

    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn are_meaningful_for(self, per_type: Self) -> bool {
        self.difference(Self::UNIVERSAL).difference(per_type).0 == 0
    }

    pub const fn attenuate(self, wanted: Self) -> Option<Self> {
        if !self.contains(wanted) {
            return None;
        }
        Some(wanted)
    }
}
