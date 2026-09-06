//! Opaque handle encoding and quota arithmetic.
//!
//! A handle combines a slot index, generation, reserved bits, and table mixer.
//! Generations reject common stale reuse; mixers make equal slots differ across
//! tables but are not authority. The kernel's non-repeating table identity is
//! the authority check (D-78, D-204).
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
/// They keep zero invalid and the encoding non-dense (D-70 rule 7).
pub const RESERVED_BITS: u32 = 2;

/// The reserved pattern itself.
pub const RESERVED: u32 = 0b11;

/// Bits of a handle naming which arena slot it is.
///
/// The global arena therefore has 262,144 slots (D-78).
pub const INDEX_BITS: u32 = 18;

/// Bits counting how many times a slot has been reused.
///
/// The counter rejects common stale handles but wraps after 4,096 generations.
pub const GENERATION_BITS: u32 = 32 - INDEX_BITS - RESERVED_BITS;

/// Slots the arena can hold.
pub const SLOTS: u32 = 1 << INDEX_BITS;

/// Generations a slot passes through before repeating.
pub const GENERATIONS: u32 = 1 << GENERATION_BITS;

/// The successor of a never-reused table identity.
///
/// The kernel stores the next identity it will issue and reserves `u64::MAX`
/// as the exhausted-state sentinel. Checked addition makes wrapping into an
/// identity that may still own arena entries unrepresentable (D-204).
pub const fn next_table_identity(identity: u64) -> Option<u64> {
    identity.checked_add(1)
}

/// A table-derived constant that usually makes the same slot look different.
///
/// It is not a secret or an authority boundary; it prevents accidental reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mixer(u32);

/// What a table number is multiplied by to become a mixer.
///
/// Being odd makes multiplication a bijection over `u32`.
const MULTIPLIER: u32 = 0x9E37_79B9;

/// What is added after the multiply, so that table zero is not a fixed point.
const ADDEND: u32 = 0x7F4A_7C15;

impl Mixer {
    /// A mixer built from `seed`.
    ///
    /// Reserved bits are cleared. Zero is the legal identity mixer; tables use
    /// [`for_table`](Self::for_table) instead.
    pub const fn new(seed: u32) -> Self {
        Self(seed & !RESERVED)
    }

    /// A mixer for table number `id`, distinct and never the identity.
    ///
    /// The odd multiply is bijective; a cleared result falls back to the odd
    /// multiplier so no table receives the identity (D-95, D-132).
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
/// This decoded value remains distinct from the encoded process-visible word.
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
    /// Saturation would make the last generation permanently reusable.
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
    /// `Some` proves only that the encoding is well formed; the arena still
    /// validates occupancy and generation.
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
/// The free function also serves table teardown, which holds only a raw counter.
pub const fn next_generation(generation: u32) -> u32 {
    // Raw counters may be `u32::MAX`; checked addition would panic in debug.
    generation.wrapping_add(1) % GENERATIONS
}

/// A table's occupancy: handles it holds, and slots promised to operations
/// that have not finished.
///
/// Admission reserves capacity before an operation can produce a handle. Live
/// plus reserved slots are bounded together, with both counters packed into
/// one word for an atomic transition (D-78).
///
/// # Examples
///
/// ```
/// use handle_space::Quota;
///
/// let limit = 2;
/// let empty = Quota::EMPTY;
/// assert_eq!(Quota::live(empty), 0);
///
/// // Admit an operation that may produce a handle.
/// let promised = Quota::reserve(empty, limit).expect("room for one");
/// assert_eq!(Quota::reserved(promised), 1);
///
/// // The handle arrives: the promise becomes an occupant.
/// let held = Quota::commit(promised).expect("a promise to keep");
/// assert_eq!((Quota::live(held), Quota::reserved(held)), (1, 0));
/// ```
#[derive(Debug)]
pub struct Quota;

impl Quota {
    /// A table holding nothing and promising nothing.
    pub const EMPTY: u64 = 0;

    /// Handles the table holds.
    pub const fn live(bits: u64) -> u32 {
        bits as u32
    }

    /// Slots promised to operations still in flight.
    pub const fn reserved(bits: u64) -> u32 {
        (bits >> 32) as u32
    }

    /// What the limit bounds: held plus promised.
    ///
    /// `u64` because two `u32`s can sum past one, and a limit check that
    /// wrapped would admit exactly the operation it exists to refuse.
    pub const fn charged(bits: u64) -> u64 {
        Self::live(bits) as u64 + Self::reserved(bits) as u64
    }

    /// Promise a slot to an operation being admitted.
    ///
    /// `None` when the combined quota or reserved half is full, before work is
    /// accepted (D-151).
    pub const fn reserve(bits: u64, limit: usize) -> Option<u64> {
        if Self::reserved(bits) == u32::MAX || Self::charged(bits) >= limit as u64 {
            return None;
        }
        Some(bits + (1 << 32))
    }

    /// Turn a promise into a held handle.
    ///
    /// `None` if nothing was promised, which is a caller committing twice.
    pub const fn commit(bits: u64) -> Option<u64> {
        if Self::reserved(bits) == 0 || Self::live(bits) == u32::MAX {
            return None;
        }
        Some(bits - (1 << 32) + 1)
    }

    /// Give a promise back unspent.
    pub const fn release(bits: u64) -> Option<u64> {
        if Self::reserved(bits) == 0 {
            return None;
        }
        Some(bits - (1 << 32))
    }

    /// Take a slot outright, with no promise before it.
    ///
    /// What an ordinary insert does: the caller is holding the answer already,
    /// so there is nothing to admit and nothing to refuse later.
    pub const fn take(bits: u64, limit: usize) -> Option<u64> {
        if Self::live(bits) == u32::MAX || Self::charged(bits) >= limit as u64 {
            return None;
        }
        Some(bits + 1)
    }
}

/// What a handle permits.
///
/// Rights name distinctions the kernel makes about an object, not syscall
/// names; universal rights remain separate from per-type rights (D-78).
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

    /// May read or receive this object's data.
    pub const READ: Self = Self(1 << 8);

    /// May write or send this object's data.
    pub const WRITE: Self = Self(1 << 9);

    /// The memory behind it may be executed.
    ///
    /// Never held together with [`WRITE`](Self::WRITE) in a mapping: W^X is a
    /// property of the page tables (D-12), and this right is what a caller
    /// must present to ask for it at all.
    pub const EXECUTE: Self = Self(1 << 10);

    /// May be placed into an address space.
    pub const MAP: Self = Self(1 << 11);

    /// May resolve a name inside this directory that the holder already knows.
    ///
    /// Kept distinct from [`ENUMERATE`](Self::ENUMERATE): resolving a known
    /// name does not confer directory discovery (`012` §4).
    pub const TRAVERSE: Self = Self(1 << 12);

    /// May list the names in this directory.
    pub const ENUMERATE: Self = Self(1 << 13);

    /// May change what names this directory holds.
    pub const MODIFY: Self = Self(1 << 14);

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
    ///
    /// The union avoids a second literal copy of the rights list (D-173).
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

    /// The bits themselves.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Both sets together.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Only the rights present in both sets.
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
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
    pub const fn are_meaningful_for(self, per_type: Self) -> bool {
        self.difference(Self::UNIVERSAL).difference(per_type).0 == 0
    }

    /// Attenuate to `wanted`, or `None` if that asks for more than is held.
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

    #[test]
    fn a_decoded_slot_reports_the_index_and_generation_it_holds() {
        // Distinct values expose swapped accessors (D-136).
        const INDEX: u32 = 37;
        const GENERATION: u32 = 1234;

        let mixer = Mixer::for_table(9);
        let handle = Slot::new(INDEX, GENERATION)
            .expect("inside the arena")
            .encode(mixer);
        let decoded = Slot::decode(handle, mixer).expect("round trips");

        assert_eq!(decoded.index(), INDEX, "index() gave something else");
        assert_eq!(
            decoded.generation(),
            GENERATION,
            "generation() gave something else"
        );
        assert_ne!(
            decoded.index(),
            decoded.generation(),
            "the two accessors return the same field"
        );
    }

    #[test]
    fn the_widths_tile_the_word() {
        // Pin the policy values, not only their internally consistent formulas.
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

    #[test]
    fn a_malformed_value_never_decodes() {
        for value in [0, 1, 2, 4, 8, 16, 100, u32::MAX / 2, u32::MAX - 1] {
            let broken = (value ^ MIXER.bits()) & RESERVED != RESERVED;
            assert_eq!(
                Slot::decode(value, MIXER).is_none(),
                broken,
                "{value:#x} disagreed about being well formed"
            );
        }

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

    #[test]
    fn a_handle_does_not_travel_between_tables() {
        let slot = Slot::new(3, 5).expect("within range");
        let here = Mixer::new(0x1111_1111);
        let there = Mixer::new(0x2222_2222);
        assert_ne!(slot.encode(here), slot.encode(there));

        assert_ne!(Slot::decode(slot.encode(here), there), Some(slot));
    }

    #[test]
    fn no_table_number_gets_a_mixer_that_mixes_nothing() {
        // Newton iteration derives the inverse instead of restating it.
        let mut inverse = MULTIPLIER;
        for _ in 0..5 {
            inverse = inverse.wrapping_mul(2u32.wrapping_sub(MULTIPLIER.wrapping_mul(inverse)));
        }
        assert_eq!(MULTIPLIER.wrapping_mul(inverse), 1, "not an inverse");

        for cleared in 0..=RESERVED {
            let id = cleared.wrapping_sub(ADDEND).wrapping_mul(inverse);
            assert_eq!(
                id.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND),
                cleared,
                "the inverse did not name the id landing on {cleared}"
            );
            assert!(
                Mixer::for_table(id).bits() != 0,
                "table {id} got a mixer that mixes nothing"
            );
        }

        for id in 0..1024 {
            assert!(Mixer::for_table(id).bits() != 0, "table {id}");
        }
    }

    #[test]
    fn identity_detection_reads_the_mixer_bits() {
        assert!(Mixer::new(0).is_identity());
        assert!(!Mixer::new(1 << RESERVED_BITS).is_identity());
    }

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

    #[test]
    fn a_mixer_cannot_break_the_reserved_bits() {
        for seed in [0, 1, 2, 3, 0xFFFF_FFFF, 0xDEAD_BEEF] {
            let mixer = Mixer::new(seed);
            assert_eq!(mixer.bits() & RESERVED, 0, "seed {seed:#x} kept low bits");
            let slot = Slot::new(1, 1).expect("within range");
            assert_eq!(Slot::decode(slot.encode(mixer), mixer), Some(slot));
        }
    }

    #[test]
    fn an_out_of_range_slot_is_refused() {
        assert_eq!(Slot::new(SLOTS, 0), None);
        assert_eq!(Slot::new(0, GENERATIONS), None);
        assert_eq!(Slot::new(u32::MAX, u32::MAX), None);
        assert!(Slot::new(SLOTS - 1, GENERATIONS - 1).is_some());
    }

    #[test]
    fn table_identity_exhaustion_never_wraps() {
        assert_eq!(next_table_identity(u64::MAX - 1), Some(u64::MAX));
        assert_eq!(next_table_identity(u64::MAX), None);
    }

    fn alloc_set() -> std::collections::HashSet<u32> {
        std::collections::HashSet::new()
    }
}

#[cfg(test)]
mod rights_tests {
    use super::*;

    #[test]
    fn the_two_halves_do_not_overlap() {
        // Read the domain from source instead of maintaining a second list.
        const COMPOSITE: [&str; 4] = ["NONE", "UNIVERSAL", "ALL", "EMPTY"];
        let named: Vec<&str> = include_str!("lib.rs")
            .lines()
            .filter_map(|line| line.trim().strip_prefix("pub const "))
            .filter(|rest| rest.contains(": Self = Self("))
            .filter_map(|rest| rest.split_once(':').map(|(name, _)| name))
            .filter(|name| !COMPOSITE.contains(name))
            .collect();
        assert!(named.len() >= 11, "only {} rights were read", named.len());

        let bits = |name: &str| -> u32 {
            let line = include_str!("lib.rs")
                .lines()
                .find(|line| {
                    line.trim()
                        .starts_with(&format!("pub const {name}: Self = Self("))
                })
                .unwrap_or_else(|| panic!("no {name}"));
            let inside = line
                .trim()
                .trim_start_matches(&format!("pub const {name}: Self = Self("))
                .trim_end_matches(");");
            let (shift, _) = inside
                .split_once(" << ")
                .unwrap_or_else(|| panic!("{name} is not a single bit: {inside}"));
            let _ = shift;
            let place = <u32 as core::str::FromStr>::from_str(
                inside.split(" << ").nth(1).expect("a shift").trim(),
            )
            .expect("a bit position");
            1 << place
        };

        let mut seen = 0u32;
        for name in &named {
            let bit = bits(name);
            assert_eq!(bit.count_ones(), 1, "{name} is one bit");
            assert_eq!(seen & bit, 0, "{name} shares a bit with another right");
            seen |= bit;
        }
        assert_eq!(seen, Rights::ALL.bits(), "ALL is not every right");

        assert_eq!(
            Rights::UNIVERSAL.bits() & (seen & !Rights::UNIVERSAL.bits()),
            0
        );
        assert_eq!(Rights::UNIVERSAL.bits().count_ones(), 4);
    }

    #[test]
    fn an_undefined_right_is_refused() {
        let defined = Rights::ALL.bits();
        let mut checked = 0;
        for bit in 0..32u32 {
            if defined & (1 << bit) != 0 {
                continue;
            }
            assert_eq!(
                Rights::from_bits(1 << bit),
                None,
                "bit {bit} is not a right"
            );
            checked += 1;
        }
        assert!(checked > 0, "every bit is defined, so nothing was checked");

        assert_eq!(Rights::from_bits(defined), Some(Rights::ALL));
        assert_eq!(Rights::from_bits(0), Some(Rights::NONE));
        let first_undefined = (0..32u32)
            .find(|bit| defined & (1 << bit) == 0)
            .expect("some bit is undefined");
        assert_eq!(Rights::from_bits(defined | (1 << first_undefined)), None);
    }

    #[test]
    fn attenuation_only_narrows() {
        let held = Rights::READ.union(Rights::WRITE).union(Rights::TRANSFER);

        assert_eq!(held.union(Rights::READ), held, "union removed a held right");
        assert_eq!(held.attenuate(Rights::READ), Some(Rights::READ));
        assert_eq!(held.attenuate(Rights::NONE), Some(Rights::NONE));
        assert_eq!(held.attenuate(held), Some(held));
        assert_eq!(
            held.intersection(Rights::UNIVERSAL),
            Rights::TRANSFER,
            "intersection restored or discarded a held universal right"
        );

        assert_eq!(held.attenuate(Rights::EXECUTE), None, "gained a right");
        assert_eq!(
            held.attenuate(held.union(Rights::MAP)),
            None,
            "gained a right by asking for everything plus one"
        );

        for bits in 0..=Rights::ALL.bits() {
            let Some(wanted) = Rights::from_bits(bits) else {
                continue;
            };
            if let Some(got) = held.attenuate(wanted) {
                assert!(held.contains(got), "attenuation widened {bits:#x}");
            }
        }
    }

    #[test]
    fn transfer_can_be_stripped_on_its_own() {
        let held = Rights::READ.union(Rights::TRANSFER).union(Rights::WAIT);
        let confined = held.difference(Rights::TRANSFER);

        assert!(!confined.contains(Rights::TRANSFER));
        assert!(confined.contains(Rights::READ) && confined.contains(Rights::WAIT));
        assert_eq!(held.attenuate(confined), Some(confined));
    }

    #[test]
    fn contains_means_every_one() {
        let held = Rights::READ.union(Rights::WRITE);
        assert!(held.contains(Rights::READ.union(Rights::WRITE)));
        assert!(!held.contains(Rights::READ.union(Rights::EXECUTE)));
        assert!(Rights::ALL.contains(held));
        assert!(held.contains(Rights::NONE), "everything contains nothing");
    }

    #[test]
    fn a_right_that_means_nothing_for_a_type_is_not_meaningful() {
        let none = Rights::NONE;
        assert!(Rights::UNIVERSAL.are_meaningful_for(none));
        assert!(Rights::DUPLICATE.are_meaningful_for(none));
        assert!(
            !Rights::READ.are_meaningful_for(none),
            "read means nothing here"
        );
        assert!(!Rights::ALL.are_meaningful_for(none));

        let memory = Rights::READ
            .union(Rights::WRITE)
            .union(Rights::EXECUTE)
            .union(Rights::MAP);
        assert!(
            Rights::UNIVERSAL.union(memory).are_meaningful_for(memory),
            "its own rights and the universal four"
        );
        assert!(Rights::READ.union(Rights::WAIT).are_meaningful_for(memory));
        assert!(!Rights::ALL.are_meaningful_for(memory));
        assert!(!Rights::TRAVERSE.are_meaningful_for(memory));

        let directory = Rights::TRAVERSE
            .union(Rights::ENUMERATE)
            .union(Rights::MODIFY);
        assert!(Rights::TRAVERSE.are_meaningful_for(directory));
        assert!(
            !Rights::READ.are_meaningful_for(directory),
            "a directory has no bytes"
        );
        assert!(!Rights::MAP.are_meaningful_for(directory));

        assert!(Rights::UNIVERSAL.are_meaningful_for(Rights::NONE));
    }

    #[test]
    fn a_raw_counter_at_the_top_of_the_range_still_advances() {
        assert_eq!(next_generation(u32::MAX), 0);
        assert_eq!(next_generation(GENERATIONS - 1), 0);
        assert_eq!(next_generation(GENERATIONS), 1);
    }
}

#[cfg(test)]
mod quota_tests {
    use super::Quota;

    #[test]
    fn a_reserved_slot_and_a_taken_slot_cost_the_same() {
        let limit = 4;

        let reserved = Quota::reserve(Quota::EMPTY, limit).expect("room");
        let committed = Quota::commit(reserved).expect("a promise to keep");

        let taken = Quota::take(Quota::EMPTY, limit).expect("room");

        assert_eq!(committed, taken);
        assert_eq!(Quota::live(committed), 1);
        assert_eq!(Quota::reserved(committed), 0);
    }

    #[test]
    fn a_promise_holds_the_last_slot_against_an_insert() {
        let limit = 1;

        let promised = Quota::reserve(Quota::EMPTY, limit).expect("the one slot");
        assert_eq!(Quota::live(promised), 0, "no handle exists yet");

        assert!(
            Quota::take(promised, limit).is_none(),
            "an insert took the slot an admitted operation was promised"
        );
        assert!(
            Quota::reserve(promised, limit).is_none(),
            "a second operation was promised the same slot"
        );

        let released = Quota::release(promised).expect("a promise to give back");
        assert_eq!(released, Quota::EMPTY);
        assert!(Quota::take(released, limit).is_some());
    }

    #[test]
    fn releasing_a_promise_leaves_the_held_handles_alone() {
        let limit = 8;
        let held = Quota::take(Quota::take(Quota::EMPTY, limit).unwrap(), limit).unwrap();
        let promised = Quota::reserve(held, limit).expect("room");

        let released = Quota::release(promised).expect("a promise to give back");
        assert_eq!(Quota::live(released), 2);
        assert_eq!(Quota::reserved(released), 0);

        let committed = Quota::commit(promised).expect("a promise to keep");
        assert_eq!(Quota::live(committed), 3);
        assert_eq!(Quota::reserved(committed), 0);
    }

    #[test]
    fn spending_a_promise_that_was_not_made_is_refused() {
        assert!(Quota::commit(Quota::EMPTY).is_none());
        assert!(Quota::release(Quota::EMPTY).is_none());

        let held = Quota::take(Quota::EMPTY, 4).expect("room");
        assert!(Quota::commit(held).is_none(), "nothing was promised");
        assert!(Quota::release(held).is_none(), "nothing was promised");
    }

    #[test]
    fn two_counts_that_sum_past_a_word_do_not_wrap_the_limit_check() {
        let brimming = u64::from(u32::MAX) | (u64::from(u32::MAX) << 32);

        assert_eq!(Quota::charged(brimming), u64::from(u32::MAX) * 2);
        assert!(
            Quota::reserve(brimming, usize::MAX).is_none(),
            "a full table admitted an operation"
        );
        assert!(Quota::take(brimming, usize::MAX).is_none());
    }

    #[test]
    fn the_two_counts_do_not_read_each_others_bits() {
        let limit = 16;
        let mut bits = Quota::EMPTY;
        for _ in 0..5 {
            bits = Quota::take(bits, limit).expect("room");
        }
        for _ in 0..3 {
            bits = Quota::reserve(bits, limit).expect("room");
        }

        assert_eq!(Quota::live(bits), 5);
        assert_eq!(Quota::reserved(bits), 3);
        assert_eq!(Quota::charged(bits), 8);
    }
}
