//! How a handle value is made, and what it is evidence of.
//!
//! A process's power is exactly its handle table (`002` pillar 1), so the
//! value a process holds is the most security-sensitive integer in this
//! kernel. D-78 settled its shape; this crate is the arithmetic, beside
//! `port_space` and `frame_space`, where `cargo test` can reach it. The arena
//! and the lock are the kernel's.
//!
//! Three properties, each with a reason that is not obvious:
//!
//! - **The low two bits are reserved and always set.** That is what makes the
//!   encoding *non-dense*, and a non-dense encoding is what lets the table
//!   refuse to promise "lowest free handle" — the promise that serialises
//!   allocation and, per the Scalable Commutativity Rule, is the difference
//!   between an interface that scales and one that cannot. sv6 measured the
//!   unordered alternative scaling linearly with **no single-core penalty**.
//!   It also means a small integer like `3` or `0` is never a valid handle,
//!   so the commonest way to fabricate one by accident fails closed.
//! - **A generation counter shares the word with the index.** Reusing a slot
//!   changes the value, so a handle kept across a close does not name whatever
//!   took the slot next. That is use-after-free turned into a decode failure.
//! - **The value is mixed with a per-table constant.** Two processes holding
//!   the same slot see different integers, so a value guessed or leaked from
//!   one is not a handle in another.
//!
//! # Examples
//!
//! ```
//! use handle_space::{Mixer, Slot};
//!
//! let mixer = Mixer::new(0x5A5A_1234);
//! let slot = Slot::new(7, 1).expect("a slot within range");
//!
//! let handle = slot.encode(mixer);
//! assert_eq!(Slot::decode(handle, mixer), Some(slot));
//!
//! // The same slot, one generation later, is a different handle.
//! let reused = Slot::new(7, 2).expect("a slot within range");
//! assert_ne!(reused.encode(mixer), handle);
//!
//! // And the same slot in another table is a different handle again.
//! assert_ne!(slot.encode(Mixer::new(0xFFFF_0000)), handle);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Bits at the bottom of every handle, always set, never part of the slot.
///
/// Zircon's `0b11`, and the reason is not tagging: it is that an encoding with
/// reserved bits cannot be dense, so no implementation of this interface can
/// promise the lowest free handle even if a later author wanted to (D-70 rule
/// 7). The constraint is in the format rather than in a comment.
pub const RESERVED_BITS: u32 = 2;

/// The reserved pattern itself.
pub const RESERVED: u32 = 0b11;

/// Bits of a handle naming which arena slot it is.
///
/// 262,144 live handles across the system. The arena is global (D-78), so this
/// is a system-wide ceiling rather than a per-process one, and it is a number
/// to revisit with evidence rather than a format promise — every width here is
/// checked by a host test rather than assumed.
pub const INDEX_BITS: u32 = 18;

/// Bits counting how many times a slot has been reused.
///
/// 4,096 reuses before a slot's values repeat. That is not a security boundary
/// on its own — it wraps — but it turns the overwhelmingly common
/// use-after-close into an immediate decode failure rather than a silent
/// reference to someone else's object.
pub const GENERATION_BITS: u32 = 32 - INDEX_BITS - RESERVED_BITS;

/// Slots the arena can hold.
pub const SLOTS: u32 = 1 << INDEX_BITS;

/// Generations a slot passes through before repeating.
pub const GENERATIONS: u32 = 1 << GENERATION_BITS;

/// A per-table constant that makes the same slot look different in each table.
///
/// Fuchsia mixes so that a handle value observed in one process is not a
/// handle in another. It is not a secret and does not have to be: it removes
/// the *accidental* case, where a value that leaked through a log or a shared
/// buffer happens to name something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mixer(u32);

impl Mixer {
    /// A mixer built from `seed`.
    ///
    /// The low [`RESERVED_BITS`] are cleared, because mixing them would break
    /// the one property every decode checks first.
    pub const fn new(seed: u32) -> Self {
        Self(seed & !RESERVED)
    }

    /// The constant itself.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Which arena slot a handle names, and which use of it.
///
/// Not a handle: this is what a handle *decodes to*, and the kernel's side of
/// the table is indexed by it. Keeping the two types apart is what stops a
/// slot index being passed where a handle value belongs, which is the mistake
/// a plain `u32` on both sides invites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    index: u32,
    generation: u32,
}

impl Slot {
    /// A slot, or `None` if either half is out of range.
    pub const fn new(index: u32, generation: u32) -> Option<Self> {
        if index >= SLOTS || generation >= GENERATIONS {
            return None;
        }
        Some(Self { index, generation })
    }

    /// Which slot of the arena.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Which use of that slot.
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// The next use of this slot, wrapping when the counter is exhausted.
    ///
    /// Wrapping rather than saturating on purpose: a saturating counter would
    /// make one slot's last generation permanent, so every later handle for it
    /// would decode as valid forever — which is the failure this counter
    /// exists to prevent, made permanent instead of transient.
    pub const fn reused(self) -> Self {
        Self {
            index: self.index,
            generation: (self.generation + 1) % GENERATIONS,
        }
    }

    /// The value a process holds for this slot in the table `mixer` belongs to.
    pub const fn encode(self, mixer: Mixer) -> u32 {
        let base = (self.generation << INDEX_BITS) | self.index;
        ((base << RESERVED_BITS) | RESERVED) ^ mixer.bits()
    }

    /// What `value` names in the table `mixer` belongs to, if anything.
    ///
    /// `None` when the reserved bits are not intact, which is every value that
    /// was not produced by [`encode`](Self::encode) with this mixer — including
    /// zero, every small integer, and any handle from another table whose
    /// mixer differs in those bits.
    ///
    /// A `Some` here is **not** proof the handle is live. It says the value is
    /// well formed and names a slot; whether that slot currently holds
    /// anything, and whether its generation matches, is the arena's to answer.
    pub const fn decode(value: u32, mixer: Mixer) -> Option<Self> {
        let base = value ^ mixer.bits();
        if base & RESERVED != RESERVED {
            return None;
        }
        let base = base >> RESERVED_BITS;
        Self::new(base & (SLOTS - 1), base >> INDEX_BITS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mixer that is not zero, so the mixing is actually exercised.
    const MIXER: Mixer = Mixer::new(0x5A5A_1234);

    /// The widths tile a 32-bit word exactly, with nothing spare and nothing
    /// overlapping.
    ///
    /// Everything else here rests on this, and a silent overlap would make two
    /// different slots encode to one handle.
    #[test]
    fn the_widths_tile_the_word() {
        assert_eq!(RESERVED_BITS + INDEX_BITS + GENERATION_BITS, 32);
        assert_eq!(RESERVED, (1 << RESERVED_BITS) - 1);
        assert_eq!(SLOTS, 1 << INDEX_BITS);
        assert_eq!(GENERATIONS, 1 << GENERATION_BITS);
    }

    /// Every slot round-trips, at the edges of both fields.
    #[test]
    fn a_slot_round_trips() {
        let corners = [0, 1, SLOTS / 2, SLOTS - 2, SLOTS - 1];
        let ages = [0, 1, GENERATIONS / 2, GENERATIONS - 2, GENERATIONS - 1];
        for index in corners {
            for generation in ages {
                let slot = Slot::new(index, generation).expect("within range");
                assert_eq!(
                    Slot::decode(slot.encode(MIXER), MIXER),
                    Some(slot),
                    "slot {index}/{generation} did not survive a round trip"
                );
            }
        }
    }

    /// **The invalid value never decodes** — the first of the two checks D-28
    /// singles out as most likely to be silently unfailable.
    ///
    /// Every value whose reserved bits are not intact must be refused, and the
    /// ones that matter are the ones a bug produces by accident: zero, a small
    /// count, a length, a negative-looking word.
    #[test]
    fn a_malformed_value_never_decodes() {
        for value in [0, 1, 2, 4, 8, 16, 100, u32::MAX / 2, u32::MAX - 1] {
            let raw = value ^ MIXER.bits();
            if raw & RESERVED == RESERVED {
                continue;
            }
            assert_eq!(
                Slot::decode(value, MIXER),
                None,
                "{value:#x} decoded despite broken reserved bits"
            );
        }

        // And exhaustively over the low bits, which is where the property is.
        for low in 0..(1u32 << 8) {
            let value = low ^ MIXER.bits();
            let intact = low & RESERVED == RESERVED;
            assert_eq!(
                Slot::decode(value, MIXER).is_some(),
                intact,
                "low bits {low:#b} disagreed about being well formed"
            );
        }
    }

    /// Zero is never a handle, whatever the mixer.
    ///
    /// Worth its own check because zero is what uninitialised memory, a failed
    /// allocation and a forgotten field all produce.
    #[test]
    fn zero_is_never_a_handle() {
        for seed in [0, 1, 0xFFFF_FFFF, 0x5A5A_1234, 0xDEAD_BEEF] {
            let mixer = Mixer::new(seed);
            assert_eq!(
                Slot::decode(0, mixer),
                None,
                "zero decoded under mixer {seed:#x}"
            );
        }
    }

    /// **A reused slot never yields the same value twice** — the second of
    /// D-28's two, and the one the generation counter exists for.
    ///
    /// Walked over a whole generation cycle for one slot: every value distinct
    /// until the counter wraps, and the wrap is where it repeats.
    #[test]
    fn a_reused_slot_never_repeats_before_it_wraps() {
        let mut slot = Slot::new(9, 0).expect("within range");
        let first = slot.encode(MIXER);
        let mut seen = alloc_set();

        for step in 0..GENERATIONS {
            let value = slot.encode(MIXER);
            assert!(
                seen.insert(value),
                "slot 9 repeated a handle after {step} reuses"
            );
            slot = slot.reused();
        }

        assert_eq!(
            slot.generation(),
            0,
            "the counter did not wrap where it says"
        );
        assert_eq!(slot.encode(MIXER), first, "the wrap is where it repeats");
    }

    /// Two different slots never share a handle, whatever the mixer.
    #[test]
    fn no_two_slots_share_a_handle() {
        let mut seen = alloc_set();
        for index in [0, 1, 2, SLOTS - 1] {
            for generation in [0, 1, GENERATIONS - 1] {
                let slot = Slot::new(index, generation).expect("within range");
                assert!(
                    seen.insert(slot.encode(MIXER)),
                    "slot {index}/{generation} collided with an earlier one"
                );
            }
        }
    }

    /// The same slot in two tables is two different handles.
    #[test]
    fn a_handle_does_not_travel_between_tables() {
        let slot = Slot::new(3, 5).expect("within range");
        let here = Mixer::new(0x1111_1111);
        let there = Mixer::new(0x2222_2222);
        assert_ne!(slot.encode(here), slot.encode(there));

        // And decoding one in the other names something else, or nothing —
        // never the same slot.
        assert_ne!(Slot::decode(slot.encode(here), there), Some(slot));
    }

    /// A mixer cannot disturb the reserved bits, whatever seed it is given.
    ///
    /// If it could, a table would exist whose every handle failed to decode —
    /// or, worse, one where malformed values decoded fine.
    #[test]
    fn a_mixer_cannot_break_the_reserved_bits() {
        for seed in [0, 1, 2, 3, 0xFFFF_FFFF, 0xDEAD_BEEF] {
            let mixer = Mixer::new(seed);
            assert_eq!(mixer.bits() & RESERVED, 0, "seed {seed:#x} kept low bits");
            let slot = Slot::new(1, 1).expect("within range");
            assert_eq!(Slot::decode(slot.encode(mixer), mixer), Some(slot));
        }
    }

    /// Out-of-range halves are refused rather than silently truncated.
    #[test]
    fn an_out_of_range_slot_is_refused() {
        assert_eq!(Slot::new(SLOTS, 0), None);
        assert_eq!(Slot::new(0, GENERATIONS), None);
        assert_eq!(Slot::new(u32::MAX, u32::MAX), None);
        assert!(Slot::new(SLOTS - 1, GENERATIONS - 1).is_some());
    }

    /// A tiny set, so the crate needs no dependency to test uniqueness.
    fn alloc_set() -> std::collections::HashSet<u32> {
        std::collections::HashSet::new()
    }
}
