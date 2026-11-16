pub const MAX_COLOURS: usize = u64::BITS as usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Colours(usize);

impl Colours {
    pub const ONE: Self = Self(1);

    pub const fn new(count: u32) -> Option<Self> {
        let count = count as usize;
        if count.is_power_of_two() && count <= MAX_COLOURS {
            Some(Self(count))
        } else {
            None
        }
    }

    pub const fn count(self) -> usize {
        self.0
    }

    pub const fn of(self, frame: usize) -> usize {
        frame & (self.0 - 1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColourMask(u64);

impl ColourMask {
    pub(crate) const NONE: Self = Self(0);

    pub const fn full(colours: Colours) -> Self {
        Self(u64::MAX >> (MAX_COLOURS - colours.count()))
    }

    pub const fn new(colours: Colours, bits: u64) -> Option<Self> {
        if bits == 0 || bits & !Self::full(colours).0 != 0 {
            return None;
        }
        Some(Self(bits))
    }

    pub const fn contains(self, colour: usize) -> bool {
        colour < MAX_COLOURS && self.0 & (1 << colour) != 0
    }

    pub const fn is_within(self, parent: Self) -> bool {
        self.0 & !parent.0 == 0
    }

    // A colour count divides 64, so bit `b` of every bitmap word is frame colour `b mod count`.
    pub(crate) const fn word_pattern(self, colours: Colours) -> u64 {
        let mut pattern = self.0 & Self::full(colours).0;
        let mut width = colours.count();
        while width < MAX_COLOURS {
            pattern |= pattern << width;
            width *= 2;
        }
        pattern
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColourCounts([usize; MAX_COLOURS]);

impl ColourCounts {
    pub const ZERO: Self = Self([0; MAX_COLOURS]);

    pub const fn get(&self, colour: usize) -> usize {
        if colour < MAX_COLOURS {
            self.0[colour]
        } else {
            0
        }
    }

    #[must_use = "a refused count changed nothing"]
    pub const fn set(&mut self, colour: usize, count: usize) -> bool {
        if colour >= MAX_COLOURS {
            return false;
        }
        self.0[colour] = count;
        true
    }

    // Callers bound `by` first, so neither step saturates.
    pub(crate) fn increase(&mut self, colour: usize, by: usize) {
        if let Some(count) = self.0.get_mut(colour) {
            *count = count.saturating_add(by);
        }
    }

    pub(crate) fn decrease(&mut self, colour: usize, by: usize) {
        if let Some(count) = self.0.get_mut(colour) {
            *count = count.saturating_sub(by);
        }
    }

    pub(crate) const fn is_zero(&self) -> bool {
        let mut colour = 0;
        while colour < MAX_COLOURS {
            if self.0[colour] != 0 {
                return false;
            }
            colour += 1;
        }
        true
    }
}
