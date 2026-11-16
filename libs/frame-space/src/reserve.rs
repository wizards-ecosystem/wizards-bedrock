use crate::{ColourCounts, ColourMask, Colours, FrameSpace, FreeError, MAX_COLOURS};

// Per colour, every reservation's unused share summed, never above that colour's free frames.
#[derive(Debug)]
pub struct Pool {
    unused: ColourCounts,
}

impl Pool {
    pub const fn new() -> Self {
        Self {
            unused: ColourCounts::ZERO,
        }
    }

    // Leaves the reservation empty, so it can release nothing twice.
    #[must_use = "a refused dissolve released nothing"]
    pub fn dissolve(&mut self, reservation: &mut Reservation) -> bool {
        if !reservation.used.is_zero() {
            return false;
        }
        for colour in 0..MAX_COLOURS {
            self.unused
                .decrease(colour, reservation.reserved.get(colour));
        }
        reservation.reserved = ColourCounts::ZERO;
        true
    }

    fn unreserved<const WORDS: usize>(
        &self,
        space: &FrameSpace<WORDS>,
        colours: Colours,
        mask: ColourMask,
    ) -> Option<ColourMask> {
        let bits = (0..colours.count())
            .filter(|&colour| {
                mask.contains(colour) && space.free_in(colours, colour) > self.unused.get(colour)
            })
            .fold(0, |bits, colour| bits | 1 << colour);
        ColourMask::new(colours, bits)
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

// `used` never exceeds `reserved`, and both are zero outside `mask`.
#[derive(Debug)]
pub struct Reservation {
    mask: ColourMask,
    reserved: ColourCounts,
    used: ColourCounts,
}

impl Reservation {
    pub const fn new(mask: ColourMask) -> Self {
        Self {
            mask,
            reserved: ColourCounts::ZERO,
            used: ColourCounts::ZERO,
        }
    }

    const fn unused(&self, colour: usize) -> usize {
        self.reserved
            .get(colour)
            .saturating_sub(self.used.get(colour))
    }

    // Moves reservation to a child whose mask lies within this one; a refusal changes neither.
    #[must_use = "a refused transfer moved nothing"]
    pub fn transfer(&mut self, child: &mut Self, amounts: &ColourCounts) -> bool {
        if !child.mask.is_within(self.mask) {
            return false;
        }
        let admitted = (0..MAX_COLOURS).all(|colour| {
            let amount = amounts.get(colour);
            amount == 0
                || (child.mask.contains(colour)
                    && amount <= self.unused(colour)
                    && child.reserved.get(colour).checked_add(amount).is_some())
        });
        if !admitted {
            return false;
        }
        for colour in 0..MAX_COLOURS {
            self.reserved.decrease(colour, amounts.get(colour));
            child.reserved.increase(colour, amounts.get(colour));
        }
        true
    }

    // Leaves this reservation empty, so it can return nothing twice.
    #[must_use = "a refused return moved nothing"]
    pub fn return_to(&mut self, parent: &mut Self) -> bool {
        if !self.used.is_zero() || !self.mask.is_within(parent.mask) {
            return false;
        }
        for colour in 0..MAX_COLOURS {
            parent.reserved.increase(colour, self.reserved.get(colour));
        }
        self.reserved = ColourCounts::ZERO;
        true
    }

    fn chargeable(&self, colours: Colours) -> Option<ColourMask> {
        let bits = (0..colours.count())
            .filter(|&colour| self.unused(colour) > 0)
            .fold(0, |bits, colour| bits | 1 << colour);
        ColourMask::new(colours, bits)
    }
}

impl<const WORDS: usize> FrameSpace<WORDS> {
    #[must_use = "a refused reservation reserved nothing"]
    pub fn reserve(
        &self,
        colours: Colours,
        pool: &mut Pool,
        into: &mut Reservation,
        amounts: &ColourCounts,
    ) -> bool {
        let admitted = (0..MAX_COLOURS).all(|colour| {
            let amount = amounts.get(colour);
            let slack = self
                .free_in(colours, colour)
                .saturating_sub(pool.unused.get(colour));
            amount == 0
                || (into.mask.contains(colour)
                    && amount <= slack
                    && into.reserved.get(colour).checked_add(amount).is_some())
        });
        if !admitted {
            return false;
        }
        for colour in 0..MAX_COLOURS {
            pool.unused.increase(colour, amounts.get(colour));
            into.reserved.increase(colour, amounts.get(colour));
        }
        true
    }

    pub fn allocate_unreserved(
        &mut self,
        colours: Colours,
        pool: &Pool,
        mask: ColourMask,
    ) -> Option<usize> {
        let mask = pool.unreserved(self, colours, mask)?;
        self.allocate_coloured(colours, mask)
    }

    pub fn allocate_reserved(
        &mut self,
        colours: Colours,
        pool: &mut Pool,
        into: &mut Reservation,
    ) -> Option<usize> {
        let mask = into.chargeable(colours)?;
        let frame = self.allocate_coloured(colours, mask)?;
        let colour = colours.of(frame);
        into.used.increase(colour, 1);
        pool.unused.decrease(colour, 1);
        Some(frame)
    }

    pub fn free_reserved(
        &mut self,
        frame: usize,
        colours: Colours,
        pool: &mut Pool,
        from: &mut Reservation,
    ) -> Result<(), FreeError> {
        self.check_releasable(frame)?;
        let colour = colours.of(frame);
        if from.used.get(colour) == 0 {
            return Err(FreeError::NotCharged(frame));
        }
        self.release(frame);
        from.used.decrease(colour, 1);
        pool.unused.increase(colour, 1);
        Ok(())
    }
}
