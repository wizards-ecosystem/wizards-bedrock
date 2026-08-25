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
//!   Zero is never a handle, whatever the mixer, which closes the commonest
//!   way to fabricate one by accident. Small integers with the reserved bits
//!   set — `3`, `7`, `11` — *do* decode, to slots that are almost never in
//!   the arena; it is the bounds and generation checks that refuse them, not
//!   the encoding. An earlier version of this sentence claimed otherwise
//!   (D-95).
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

/// What a table number is multiplied by to become a mixer.
///
/// The golden ratio scaled to thirty-two bits, and **odd**, which is the part
/// that matters: an odd multiplier makes the map a bijection, so two table
/// numbers cannot produce the same mixer before the reserved bits are cleared.
const MULTIPLIER: u32 = 0x9E37_79B9;

/// What is added after the multiply, so that table zero is not a fixed point.
const ADDEND: u32 = 0x7F4A_7C15;

impl Mixer {
    /// A mixer built from `seed`.
    ///
    /// The low [`RESERVED_BITS`] are cleared, because mixing them would break
    /// the one property every decode checks first.
    ///
    /// A seed of zero yields the identity, which is a mixer that mixes
    /// nothing — legal, and never what a table wants. [`for_table`] is the
    /// constructor a table should use.
    ///
    /// [`for_table`]: Self::for_table
    pub const fn new(seed: u32) -> Self {
        Self(seed & !RESERVED)
    }

    /// A mixer for table number `id`, distinct and never the identity.
    ///
    /// The kernel derived this itself and got `Mixer(0)` for the first table,
    /// so its handles were unmixed and its first one was the literal integer
    /// three (D-95). The odd constant is added *after* the multiply so that
    /// `id` zero is not a fixed point.
    ///
    /// **"Never" is now true.** The multiplier is odd, so the multiply is a
    /// bijection over the whole range — which means exactly four `id`s land in
    /// `0..=RESERVED`, and [`new`](Self::new) clears those to zero. Four out of
    /// four billion, so no kernel counting tables would ever reach one; that is
    /// also what was true of `Mixer(0)` until the day it was not (D-132).
    pub const fn for_table(id: u32) -> Self {
        let mixed = Self::new(id.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND));
        if mixed.is_identity() {
            // The multiplier itself, which is odd and so survives the clearing.
            Self::new(MULTIPLIER)
        } else {
            mixed
        }
    }

    /// Whether this mixer actually mixes.
    pub const fn is_identity(self) -> bool {
        self.0 == 0
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
            generation: next_generation(self.generation),
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

/// The generation a slot moves to when its handle is closed.
///
/// Free-standing as well as on [`Slot`] because a table advancing a slot it is
/// tearing down has a raw counter and no handle to reuse — and open-coding the
/// wrap in two places is how the two stop agreeing.
pub const fn next_generation(generation: u32) -> u32 {
    // Wrapping, not checked: this takes a *raw* counter from a caller that has
    // no handle, so `u32::MAX` is a value it can be handed, and `+ 1` panics
    // on it in a debug build — which is the profile D-65's second boot runs
    // (D-105).
    generation.wrapping_add(1) % GENERATIONS
}

/// What a handle permits.
///
/// D-78: sixteen to twenty-four rights in a single `u32`, universal split from
/// per-type, under one rule that is easy to state and easy to violate:
///
/// > A right must correspond to a distinction the kernel actually makes about
/// > that object — never to a syscall name.
///
/// That is the rule WASI's rights broke, and deleting them in Preview 2 was
/// the cost. It is also why the memory rights below are exactly the page
/// permissions: the kernel really does treat a readable mapping differently
/// from a writable one, and `MAP` is separate because being allowed to *place*
/// a page in an address space is a different power from being allowed to read
/// what is already there.
///
/// The evidence for keeping the number small is one-directional. seL4 has 4,
/// Zircon 25, CloudABI 31, Capsicum about 81 — and Capsicum's outgrew a machine
/// word and broke its ABI, so `cap_rights_t` is now a multi-word struct
/// spending seven of every sixty-four bits on metadata. managarm, adding
/// fine-grained rights ten years in, landed on about fourteen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rights(u32);

impl Rights {
    /// None at all. A handle with no rights is still a reference — it keeps
    /// the object alive — and that is deliberate.
    pub const NONE: Self = Self(0);

    /// May be duplicated, at these rights or fewer.
    pub const DUPLICATE: Self = Self(1 << 0);

    /// May be sent through a channel.
    ///
    /// Strippable on its own, which is the point: removing it yields a handle
    /// the holder can use and cannot pass on, and that is one bit rather than
    /// a mechanism (D-78).
    pub const TRANSFER: Self = Self(1 << 1);

    /// May be waited on for the object's signals.
    pub const WAIT: Self = Self(1 << 2);

    /// May be asked what it is.
    pub const INSPECT: Self = Self(1 << 3);

    /// Every right that means the same thing for every object type.
    ///
    /// Zircon's `RIGHTS_BASIC`. Splitting these from per-type rights is what
    /// keeps a right's meaning from depending on what it is attached to.
    pub const UNIVERSAL: Self = Self(0b1111);

    /// The memory behind it may be read.
    pub const READ: Self = Self(1 << 8);

    /// The memory behind it may be written.
    pub const WRITE: Self = Self(1 << 9);

    /// The memory behind it may be executed.
    ///
    /// Never held together with [`WRITE`](Self::WRITE) in a mapping: W^X is a
    /// property of the page tables (D-12), and this right is what a caller
    /// must present to ask for it at all.
    pub const EXECUTE: Self = Self(1 << 10);

    /// May be placed into an address space.
    pub const MAP: Self = Self(1 << 11);

    /// Rights from raw bits, or `None` if any undefined bit is set.
    ///
    /// Undefined bits are refused rather than masked away. Masking would make
    /// a caller asking for a right this kernel does not have yet succeed
    /// quietly, and the whole point of a rights word is that what it does not
    /// grant, it denies.
    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !Self::ALL.0 != 0 {
            return None;
        }
        Some(Self(bits))
    }

    /// Every right this kernel defines.
    pub const ALL: Self = Self(0b1111 | (0b1111 << 8));

    /// The bits themselves.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Both sets together.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// This set without `other`.
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether every right in `other` is held here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether every right here is one that can mean something for an object
    /// with `per_type` as its per-type set.
    ///
    /// D-78's rule is that a right names a distinction the kernel makes about
    /// *that object*. Nothing enforced it, and the first object shipped with
    /// `READ`, `WRITE`, `EXECUTE` and `MAP` granted on an `Event` — which has
    /// no pages for any of them to mean anything about (D-95).
    pub const fn are_meaningful_for(self, per_type: Self) -> bool {
        self.difference(Self::UNIVERSAL).difference(per_type).0 == 0
    }

    /// Attenuate to `wanted`, or `None` if that asks for more than is held.
    ///
    /// The whole of rights attenuation, and it only ever narrows. Zircon's
    /// `zx_channel_write_etc` can strip rights during a transfer — including
    /// stripping `TRANSFER` itself — which costs about four percent and makes
    /// least privilege the default rather than an option (D-78).
    pub const fn attenuate(self, wanted: Self) -> Option<Self> {
        if !self.contains(wanted) {
            return None;
        }
        Some(wanted)
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
        // Transcribed, because the three assertions below are definitions:
        // `GENERATION_BITS` *is* `32 - INDEX_BITS - RESERVED_BITS`, `SLOTS`
        // *is* `1 << INDEX_BITS`, and `GENERATIONS` *is* `1 << GENERATION_BITS`
        // — so the sum is `32 == 32` and the other two are `X == X` for every
        // width. Narrowing `INDEX_BITS` to 17 survived them all (D-124).
        assert_eq!(RESERVED_BITS, 2, "the low two bits are reserved (D-78)");
        assert_eq!(INDEX_BITS, 18, "eighteen bits of arena index");
        assert_eq!(GENERATION_BITS, 12, "and twelve of generation");
        assert_eq!(SLOTS, 262_144);
        assert_eq!(GENERATIONS, 4096);

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
        // Every value whose reserved bits are broken, and the `continue` that
        // used to be here skipped exactly the ones where the property is
        // interesting — so the cases it kept were the ones that could not
        // fail (D-95). They are now asserted from the other side.
        for value in [0, 1, 2, 4, 8, 16, 100, u32::MAX / 2, u32::MAX - 1] {
            let broken = (value ^ MIXER.bits()) & RESERVED != RESERVED;
            assert_eq!(
                Slot::decode(value, MIXER).is_none(),
                broken,
                "{value:#x} disagreed about being well formed"
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

    /// **No table number gets a mixer that mixes nothing**, the four that
    /// nearly did included.
    ///
    /// D-95 is the whole reason `for_table` exists: the kernel derived its own
    /// mixer, got `Mixer(0)` for the first table, and every handle in it came
    /// out unmixed. `is_identity` was written to ask exactly that question, and
    /// **nothing called it** — nor did any test here call `for_table` at all,
    /// so D-95's fix had a name for its property and no check of it.
    ///
    /// The doc said "never the identity" and the arithmetic said almost: the
    /// multiplier is odd, so the map is a bijection, so exactly four `id`s land
    /// in `0..=RESERVED` and `new` clears them to zero (D-132).
    #[test]
    fn no_table_number_gets_a_mixer_that_mixes_nothing() {
        // The multiplier's inverse mod 2^32, by Newton's iteration: each step
        // doubles the number of correct bits, so five reach thirty-two.
        // Computed rather than written down — a constant here would only equal
        // itself, which is what D-124 was about.
        let mut inverse = MULTIPLIER;
        for _ in 0..5 {
            inverse = inverse.wrapping_mul(2u32.wrapping_sub(MULTIPLIER.wrapping_mul(inverse)));
        }
        assert_eq!(MULTIPLIER.wrapping_mul(inverse), 1, "not an inverse");

        // The four table numbers whose product lands where `new` clears.
        for cleared in 0..=RESERVED {
            let id = cleared.wrapping_sub(ADDEND).wrapping_mul(inverse);
            assert_eq!(
                id.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND),
                cleared,
                "the inverse did not name the id landing on {cleared}"
            );
            assert!(
                !Mixer::for_table(id).is_identity(),
                "table {id} got a mixer that mixes nothing"
            );
        }

        // And the ordinary ones, table zero — D-95's own — among them.
        for id in 0..1024 {
            assert!(!Mixer::for_table(id).is_identity(), "table {id}");
        }
    }

    /// Two table numbers do not share a mixer, which is what the mixing is for.
    ///
    /// Only *nearly* true, and the bound is worth stating rather than
    /// discovering: the multiply is a bijection, but `new` then clears two
    /// bits, so mixers collide four ways. That is a property of the format and
    /// not a fault — a table's identity is checked separately, and the mixer
    /// only has to remove the accidental case (D-78).
    #[test]
    fn table_numbers_spread_across_mixers() {
        let mut seen = Vec::new();
        for id in 0..256 {
            seen.push(Mixer::for_table(id).bits());
        }
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "two table numbers shared a mixer");
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

#[cfg(test)]
mod rights_tests {
    use super::*;

    /// Universal and per-type rights occupy disjoint bits, and `ALL` is
    /// exactly their union.
    ///
    /// A right that appeared in both halves would mean the same bit answered
    /// two questions, which is how a taxonomy starts drifting toward
    /// Capsicum's.
    #[test]
    fn the_two_halves_do_not_overlap() {
        let universal = [
            Rights::DUPLICATE,
            Rights::TRANSFER,
            Rights::WAIT,
            Rights::INSPECT,
        ];
        let per_type = [Rights::READ, Rights::WRITE, Rights::EXECUTE, Rights::MAP];

        let mut seen = 0u32;
        for right in universal.iter().chain(per_type.iter()) {
            assert_eq!(right.bits().count_ones(), 1, "a right is one bit");
            assert_eq!(seen & right.bits(), 0, "two rights share a bit");
            seen |= right.bits();
        }
        assert_eq!(seen, Rights::ALL.bits());

        let universal_bits = universal.iter().fold(0, |bits, r| bits | r.bits());
        assert_eq!(universal_bits, Rights::UNIVERSAL.bits());
        assert_eq!(
            universal_bits & per_type.iter().fold(0, |b, r| b | r.bits()),
            0
        );
    }

    /// An undefined bit is refused, not masked away.
    ///
    /// Masking would let a caller ask for a right this kernel does not have
    /// yet and be told yes.
    #[test]
    fn an_undefined_right_is_refused() {
        assert_eq!(Rights::from_bits(1 << 31), None);
        assert_eq!(Rights::from_bits(1 << 4), None);
        assert_eq!(Rights::from_bits(Rights::ALL.bits() | 1 << 12), None);
        assert_eq!(Rights::from_bits(Rights::ALL.bits()), Some(Rights::ALL));
        assert_eq!(Rights::from_bits(0), Some(Rights::NONE));
    }

    /// Attenuation only ever narrows, and asking for more is refused rather
    /// than clamped.
    ///
    /// Clamping would turn "give me more than I have" into a silent success,
    /// which is the shape of every privilege-escalation bug that ever looked
    /// like a convenience.
    #[test]
    fn attenuation_only_narrows() {
        let held = Rights::READ.union(Rights::WRITE).union(Rights::TRANSFER);

        assert_eq!(held.attenuate(Rights::READ), Some(Rights::READ));
        assert_eq!(held.attenuate(Rights::NONE), Some(Rights::NONE));
        assert_eq!(held.attenuate(held), Some(held));

        assert_eq!(held.attenuate(Rights::EXECUTE), None, "gained a right");
        assert_eq!(
            held.attenuate(held.union(Rights::MAP)),
            None,
            "gained a right by asking for everything plus one"
        );

        // Whatever comes back is a subset of what was held — the invariant,
        // over every combination of the defined rights.
        for bits in 0..=Rights::ALL.bits() {
            let Some(wanted) = Rights::from_bits(bits) else {
                continue;
            };
            if let Some(got) = held.attenuate(wanted) {
                assert!(held.contains(got), "attenuation widened {bits:#x}");
            }
        }
    }

    /// Stripping `TRANSFER` yields a usable, non-delegatable handle.
    ///
    /// The one-bit version of a mechanism, and worth checking on its own
    /// because it is the case D-78 cites for attenuate-on-send.
    #[test]
    fn transfer_can_be_stripped_on_its_own() {
        let held = Rights::READ.union(Rights::TRANSFER).union(Rights::WAIT);
        let confined = held.difference(Rights::TRANSFER);

        assert!(!confined.contains(Rights::TRANSFER));
        assert!(confined.contains(Rights::READ) && confined.contains(Rights::WAIT));
        assert_eq!(held.attenuate(confined), Some(confined));
    }

    /// `contains` is subset, not intersection.
    ///
    /// The mistake that makes a check pass when *any* right matches instead of
    /// all of them, which reads identically and grants everything.
    #[test]
    fn contains_means_every_one() {
        let held = Rights::READ.union(Rights::WRITE);
        assert!(held.contains(Rights::READ.union(Rights::WRITE)));
        assert!(!held.contains(Rights::READ.union(Rights::EXECUTE)));
        assert!(Rights::ALL.contains(held));
        assert!(held.contains(Rights::NONE), "everything contains nothing");
    }

    /// A right that means nothing for an object type is refused for it.
    ///
    /// The rule D-78 states and nothing checked: `READ` on an event is not a
    /// narrower permission, it is a category error.
    #[test]
    fn a_right_that_means_nothing_for_a_type_is_not_meaningful() {
        // An object with no per-type rights at all, which is what an event is.
        let none = Rights::NONE;
        assert!(Rights::UNIVERSAL.are_meaningful_for(none));
        assert!(Rights::DUPLICATE.are_meaningful_for(none));
        assert!(
            !Rights::READ.are_meaningful_for(none),
            "read means nothing here"
        );
        assert!(!Rights::ALL.are_meaningful_for(none));

        // One with the memory rights, which is what a memory object will be.
        let memory = Rights::READ
            .union(Rights::WRITE)
            .union(Rights::EXECUTE)
            .union(Rights::MAP);
        assert!(Rights::ALL.are_meaningful_for(memory));
        assert!(Rights::READ.union(Rights::WAIT).are_meaningful_for(memory));

        // Universal rights are always meaningful, which is what makes them
        // universal.
        assert!(Rights::UNIVERSAL.are_meaningful_for(Rights::NONE));
    }

    #[test]
    fn a_raw_counter_at_the_top_of_the_range_still_advances() {
        // A caller with no handle has a bare `u32`; `+ 1` panicked on this one
        // in the debug boot (D-105).
        assert_eq!(next_generation(u32::MAX), 0);
        assert_eq!(next_generation(GENERATIONS - 1), 0);
        assert_eq!(next_generation(GENERATIONS), 1);
    }
}
