#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{verus, verus_verify};

#[cfg(verus_only)]
use crate::proof;

#[verus_verify]
pub const MAX_COLOURS: usize = 64;

verus! {

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Colours(pub(crate) usize);

impl Colours {
    pub exec const ONE: Self
        ensures
            Self::ONE.spec_count() == 1,
    {
        proof {
            proof::lemma_one_is_power();
        }
        Self(1)
    }

    pub const fn new(count: u32) -> (r: Option<Self>)
        ensures
            r matches Some(colours) ==> colours.spec_count() == count,
    {
        let count = count as usize;
        if count <= MAX_COLOURS && is_power_of_two(count) {
            proof {
                proof::lemma_power_positive(count);
            }
            Some(Self(count))
        } else {
            None
        }
    }

    pub const fn count(self) -> (r: usize)
        ensures
            r == self.spec_count(),
            proof::is_power_of_two(r),
            0 < r <= 64,
    {
        proof {
            use_type_invariant(self);
        }
        self.0
    }

    pub const fn of(self, frame: usize) -> (r: usize)
        ensures
            r == self.spec_of(frame as int),
    {
        proof {
            use_type_invariant(self);
            proof::lemma_low_mask(frame, self.0);
        }
        frame & (self.0 - 1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColourMask(pub(crate) u64);

impl ColourMask {
    pub(crate) const NONE: Self = Self(0);

    pub const fn full(colours: Colours) -> (r: Self)
        ensures
            forall|colour: int| #[trigger] r.spec_contains(colour) == (0 <= colour
                < colours.spec_count()),
            r.spec_bits() == u64::MAX >> (64 - colours.spec_count()) as u64,
    {
        let count = colours.count();
        proof {
            proof::lemma_full_mask(count as u64);
        }
        Self(u64::MAX >> ((MAX_COLOURS - count) as u64))
    }

    pub const fn new(colours: Colours, bits: u64) -> (r: Option<Self>)
        ensures
            r matches Some(mask) ==> mask.spec_bits() == bits && bits != 0,
            r.is_none() ==> bits == 0 || exists|bit: u64|
                bit < 64 && #[trigger] proof::bit_set(bits, bit) && bit >= colours.spec_count(),
            r matches Some(mask) ==> forall|colour: int|
                #[trigger] mask.spec_contains(colour) ==> 0 <= colour < colours.spec_count(),
    {
        let full = Self::full(colours);
        if bits == 0 || bits & !full.0 != 0 {
            proof {
                if bits != 0 {
                    proof::lemma_outside_bit(bits, full.0);
                    let bit = choose|bit: u64| bit < 64 && #[trigger] proof::bit_set(bits, bit)
                        && !proof::bit_set(full.0, bit);
                    assert(!full.spec_contains(bit as int));
                }
            }
            return None;
        }
        proof {
            assert forall|colour: int| #[trigger] Self(bits).spec_contains(colour) implies full.spec_contains(colour) by {
                proof::lemma_subset_bit(bits, full.0, colour as u64);
            }
        }
        Some(Self(bits))
    }

    pub const fn contains(self, colour: usize) -> (r: bool)
        ensures
            r == self.spec_contains(colour as int),
    {
        if colour < MAX_COLOURS {
            proof {
                proof::lemma_test_bit(self.0, colour as u64);
            }
            self.0 & (1 << colour) != 0
        } else {
            false
        }
    }

    pub const fn is_within(self, parent: Self) -> (r: bool)
        ensures
            r ==> forall|colour: int| #[trigger] self.spec_contains(colour) ==> parent.spec_contains(colour),
    {
        proof {
            assert forall|colour: int| #[trigger] self.spec_contains(colour) && self.0 & !parent.0 == 0 implies parent.spec_contains(colour) by {
                proof::lemma_subset_bit(self.0, parent.0, colour as u64);
            }
        }
        self.0 & !parent.0 == 0
    }

    // A colour count divides 64, so bit `b` of every bitmap word is frame colour `b mod count`.
    pub(crate) const fn word_pattern(self, colours: Colours) -> (r: u64)
        ensures
            forall|bit: u64|
                bit < 64 ==> #[trigger] proof::bit_set(r, bit) == self.spec_contains(
                    bit as int % colours.spec_count() as int,
                ),
    {
        proof {
            use_type_invariant(colours);
        }
        proof {
            use_type_invariant(colours);
        }
        let mut pattern = self.0 & Self::full(colours).0;
        let mut width = colours.count();
        proof {
            colours.lemma_pattern_start(self, pattern);
        }
        while width < MAX_COLOURS
            invariant
                colours.valid(),
                colours.pattern_holds(self, pattern, width),
            decreases 64 - width,
        {
            let ghost before = pattern;
            proof {
                colours.lemma_pattern_width(width);
            }
            pattern = pattern | (pattern << (width as u64));
            proof {
                colours.lemma_pattern_doubles(self, before, width);
            }
            width *= 2;
        }
        proof {
            colours.lemma_pattern_done(self, pattern, width);
        }
        pattern
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColourCounts(pub(crate) [usize; MAX_COLOURS]);

impl ColourCounts {
    pub exec const ZERO: Self
        ensures
            forall|colour: int| #[trigger] Self::ZERO.spec_get(colour) == 0,
    {
        Self([0; MAX_COLOURS])
    }

    pub const fn get(&self, colour: usize) -> (r: usize)
        ensures
            r == self.spec_get(colour as int),
    {
        if colour < MAX_COLOURS {
            self.0[colour]
        } else {
            0
        }
    }

    #[must_use = "a refused count changed nothing"]
    pub const fn set(&mut self, colour: usize, count: usize) -> (r: bool)
        ensures
            r == (colour < 64),
            forall|other: int|
                #[trigger] final(self).spec_get(other) == if r && other == colour {
                    count
                } else {
                    old(self).spec_get(other)
                },
        no_unwind
    {
        if colour >= MAX_COLOURS {
            return false;
        }
        self.0[colour] = count;
        true
    }

    // Callers bound `by` first, so neither step saturates.
    pub(crate) fn increase(&mut self, colour: usize, by: usize)
        requires
            colour < 64,
        ensures
            final(self).0@ == old(self).0@.update(
                colour as int,
                if old(self).0@[colour as int] + by <= usize::MAX {
                    (old(self).0@[colour as int] + by) as usize
                } else {
                    usize::MAX
                },
            ),
            final(self).spec_get(colour as int) == if old(self).spec_get(colour as int) + by
                <= usize::MAX {
                (old(self).spec_get(colour as int) + by) as usize
            } else {
                usize::MAX
            },
            forall|other: int|
                other != colour ==> #[trigger] final(self).spec_get(other) == old(self).spec_get(
                    other,
                ),
        no_unwind
    {
        self.0[colour] = self.0[colour].saturating_add(by);
    }

    pub(crate) fn decrease(&mut self, colour: usize, by: usize)
        requires
            colour < 64,
        ensures
            final(self).0@ == old(self).0@.update(
                colour as int,
                if old(self).0@[colour as int] >= by {
                    (old(self).0@[colour as int] - by) as usize
                } else {
                    0
                },
            ),
            final(self).spec_get(colour as int) == if old(self).spec_get(colour as int) >= by {
                (old(self).spec_get(colour as int) - by) as usize
            } else {
                0
            },
            forall|other: int|
                other != colour ==> #[trigger] final(self).spec_get(other) == old(self).spec_get(
                    other,
                ),
        no_unwind
    {
        self.0[colour] = self.0[colour].saturating_sub(by);
    }

    pub(crate) const fn is_zero(&self) -> (r: bool)
        ensures
            r == forall|colour: int| #[trigger] self.spec_get(colour) == 0,
    {
        let mut colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
                forall|other: int| 0 <= other < colour ==> #[trigger] self.spec_get(other) == 0,
            decreases 64 - colour,
        {
            if self.0[colour] != 0 {
                proof {
                    assert(self.spec_get(colour as int) != 0);
                }
                return false;
            }
            colour += 1;
        }
        true
    }
}

const fn is_power_of_two(value: usize) -> (r: bool)
    ensures
        r == proof::is_power_of_two(value),
{
    let mut shift = 0;
    while shift < usize::BITS
        invariant
            shift <= usize::BITS,
            forall|smaller: u32| smaller < shift ==> value != 1usize << smaller,
        decreases usize::BITS - shift,
    {
        if value == 1 << shift {
            return true;
        }
        shift += 1;
    }
    false
}

} // verus!

verus! {

impl Colours {
    #[verifier::type_invariant]
    pub(crate) open spec fn valid(self) -> bool {
        proof::is_power_of_two(self.0) && 0 < self.0 <= 64
    }

    pub open(crate) spec fn spec_count(self) -> usize {
        self.0
    }

    pub open(crate) spec fn spec_of(self, frame: int) -> int {
        frame % (self.0 as int)
    }

    pub(crate) open spec fn pattern_holds(self, mask: ColourMask, pattern: u64, width: usize) -> bool {
        &&& proof::is_power_of_two(width)
        &&& self.0 <= width <= 64
        &&& width as int % self.0 as int == 0
        &&& forall|bit: u64|
            bit < 64 ==> #[trigger] proof::bit_set(pattern, bit) == (bit < width
                && mask.spec_contains(bit as int % self.0 as int))
    }

    pub(crate) proof fn lemma_pattern_start(self, mask: ColourMask, pattern: u64)
        requires
            self.valid(),
            pattern == mask.0 & (u64::MAX >> (64 - self.0) as u64),
        ensures
            self.pattern_holds(mask, pattern, self.0),
    {
        proof::lemma_full_mask(self.0 as u64);
        vstd::arithmetic::div_mod::lemma_mod_self_0(self.0 as int);
        assert forall|bit: u64| bit < 64 implies #[trigger] proof::bit_set(pattern, bit) == (bit
            < self.0 && mask.spec_contains(bit as int % self.0 as int)) by {
            proof::lemma_and_bit(mask.0, u64::MAX >> (64 - self.0) as u64, bit);
            if bit < self.0 {
                vstd::arithmetic::div_mod::lemma_small_mod(bit as nat, self.0 as nat);
            }
        }
    }

    pub(crate) proof fn lemma_pattern_width(self, width: usize)
        requires
            proof::is_power_of_two(width),
            width < 64,
        ensures
            width * 2 <= 64,
            0 < width,
    {
        let shift = choose|shift: u32| shift < usize::BITS && width == 1usize << shift;
        assert(shift < usize::BITS && (1usize << shift) < 64 ==> (1usize << shift) * 2 <= 64 && 0
            < (1usize << shift)) by (bit_vector);
    }

    pub(crate) proof fn lemma_pattern_doubles(self, mask: ColourMask, before: u64, width: usize)
        requires
            self.valid(),
            self.pattern_holds(mask, before, width),
            width < 64,
        ensures
            self.pattern_holds(mask, before | (before << width as u64), (width * 2) as usize),
    {
        self.lemma_pattern_width(width);
        let count = self.0 as int;
        let after = before | (before << width as u64);
        vstd::arithmetic::div_mod::lemma_fundamental_div_mod(width as int, count);
        let multiple = width as int / count;
        assert forall|bit: u64| bit < 64 implies #[trigger] proof::bit_set(after, bit) == (bit < width
            * 2 && mask.spec_contains(bit as int % count)) by {
            proof::lemma_shift_or(before, width as u64, bit);
            if bit >= width {
                let low = (bit - width) as u64;
                assert(bit as int == count * multiple + low as int);
                vstd::arithmetic::div_mod::lemma_mod_multiples_vanish(multiple, low as int, count);
            }
        }
        let shift = choose|shift: u32| shift < usize::BITS && width == 1usize << shift;
        assert((1usize << shift) * 2 == 1usize << (shift + 1) as u32 && shift < 6) by (bit_vector)
            requires
                shift < usize::BITS,
                (1usize << shift) < 64,
        ;
        assert(proof::is_power_of_two((width * 2) as usize)) by {
            assert((width * 2) as usize == 1usize << (shift + 1) as u32);
        }
        assert((width * 2) as int % count == 0) by {
            assert((width * 2) as int == (2 * multiple) * count) by (nonlinear_arith)
                requires
                    width as int == count * multiple,
            ;
            vstd::arithmetic::div_mod::lemma_mod_multiples_basic(2 * multiple, count);
        }
    }

    pub(crate) proof fn lemma_pattern_done(self, mask: ColourMask, pattern: u64, width: usize)
        requires
            self.pattern_holds(mask, pattern, width),
            width >= 64,
        ensures
            forall|bit: u64|
                bit < 64 ==> #[trigger] proof::bit_set(pattern, bit) == mask.spec_contains(
                    bit as int % self.0 as int,
                ),
    {
    }

    // A frame's bitmap position has the frame's colour, since a colour count divides 64.
    pub(crate) proof fn lemma_pattern(self, mask: ColourMask, pattern: u64, frame: int)
        requires
            self.valid(),
            0 <= frame,
            forall|bit: u64|
                bit < 64 ==> #[trigger] proof::bit_set(pattern, bit) == mask.spec_contains(
                    bit as int % self.0 as int,
                ),
        ensures
            proof::bit_set(pattern, (frame % 64) as u64) == mask.spec_contains(self.spec_of(frame)),
    {
        self.lemma_colour_of_position(frame);
        assert(proof::bit_set(pattern, (frame % 64) as u64) == mask.spec_contains(
            ((frame % 64) as u64) as int % self.0 as int,
        ));
    }

    pub(crate) proof fn lemma_colour_of_position(self, frame: int)
        requires
            self.valid(),
            0 <= frame,
        ensures
            (frame % 64) % (self.0 as int) == self.spec_of(frame),
    {
        let shift = choose|shift: u32| shift < usize::BITS && self.0 == 1usize << shift;
        assert(shift < usize::BITS && (1usize << shift) <= 64 ==> 64usize % (1usize << shift) == 0)
            by (bit_vector);
        vstd::arithmetic::div_mod::lemma_fundamental_div_mod(64, self.0 as int);
        let other = 64int / (self.0 as int);
        assert((self.0 as int) * other == 64);
        vstd::arithmetic::div_mod::lemma_mod_mod(frame, self.0 as int, other);
    }
}

impl ColourMask {
    pub open(crate) spec fn spec_bits(self) -> u64 {
        self.0
    }

    pub open(crate) spec fn spec_contains(self, colour: int) -> bool {
        0 <= colour < 64 && proof::bit_set(self.0, colour as u64)
    }
}

impl ColourCounts {
    pub open(crate) spec fn spec_get(self, colour: int) -> usize {
        if 0 <= colour < 64 {
            self.0@[colour]
        } else {
            0
        }
    }
}

} // verus!
