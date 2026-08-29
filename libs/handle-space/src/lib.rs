//! How a handle value is made, and what it is evidence of.
//!
//! A process's external-object authority is exactly its handle table (`002`
//! pillar 1), so the value a process holds is the most security-sensitive
//! integer in this kernel. D-78 settled its shape; this crate is the arithmetic,
//! beside `port_space` and `frame_space`, where `cargo test` can reach it. The
//! arena and the lock are the kernel's.
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
//! - **The value is mixed with a table-derived constant.** Nearby processes
//!   holding the same slot see different integers, so a value guessed or
//!   leaked from one is usually not even well formed in another. Mixers may
//!   repeat; the kernel's non-repeating owner identity is the authority check
//!   (D-204).
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

/// A table's occupancy: handles it holds, and slots promised to operations
/// that have not finished.
///
/// D-78 made the quota the answer to kernel-memory exhaustion at the handle
/// layer. Phase 6 makes an operation able to *produce* a handle, and that
/// splits the count in two: a submission admitted now will need a slot later,
/// and the only moment a program can be told "no room" and do something about
/// it is at submission. So a slot is **reserved** when the operation is
/// admitted and **committed** when the handle arrives, and the sum of the two
/// is what the limit bounds.
///
/// Both counts live in one word so the transition between them is a single
/// compare-and-exchange, which is what lets the kernel hold this in an atomic
/// rather than behind a lock. The arithmetic is here because it is arithmetic,
/// and because `reserve`-then-never-`release` is a leak that a host test can
/// find and a machine cannot.
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
    /// `None` when the table is full — which is the answer the program gets at
    /// *submission*, the one moment it can choose a different operation.
    /// `IORING_FILE_INDEX_ALLOC` reporting `-ENFILE` at completion instead is
    /// the failure this shape exists to avoid (D-151).
    /// The half being incremented is checked as well as the limit. A limit
    /// larger than a `u32` cannot bound either count on its own, and the
    /// addition below would carry a full promise half into the held half —
    /// turning a full table into an empty one with a billion handles. Not a
    /// state this kernel's limit of 64 can reach, which is why the arithmetic
    /// has to be right on values it will not produce.
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
    ///
    /// The refused path, the cancelled path and the dead-program path all end
    /// here, and there is no fourth.
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

    /// Give a held handle back.
    ///
    /// `None` if the table held none, which is a double close.
    pub const fn close(bits: u64) -> Option<u64> {
        if Self::live(bits) == 0 {
            return None;
        }
        Some(bits - 1)
    }
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
/// the cost. `READ` and `WRITE` describe object data; the concrete operation is
/// type-specific, such as file bytes or channel receive/send. `EXECUTE` and
/// `MAP` reserve the distinct future powers to execute bytes and place them in
/// an address space.
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
    /// **Distinct from [`ENUMERATE`](Self::ENUMERATE) on purpose**, and it is
    /// the split `012` §4 calls the valuable one: "may resolve a name I know"
    /// and "may list the names" are different powers, and they are a real
    /// kernel distinction by D-78's rule because the *resolver* consults one
    /// and `readdir` consults the other.
    ///
    /// Capsicum folds enumeration into read. WASI Preview 1 had both by
    /// accident of being syscall-named, which is the failure D-78 exists to
    /// prevent — these two are separate because the kernel behaves
    /// differently, not because there are two calls.
    pub const TRAVERSE: Self = Self(1 << 12);

    /// May list the names in this directory.
    pub const ENUMERATE: Self = Self(1 << 13);

    /// May change what names this directory holds.
    ///
    /// Nothing exercises it: this phase has no write path, and `014` carries
    /// that as its largest deliberate hole. The bit is defined because
    /// *leaving a gap* in a rights word is how a later addition renumbers the
    /// ones above it — and D-78 requires the encoding to let an object type be
    /// added without renumbering.
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
    /// The universal four, then the per-type ones. Written as the union of the
    /// constants rather than as a literal, because a literal is a second copy
    /// of the list and goes stale the moment a right is added — which is what
    /// happened when the directory rights arrived (D-173).
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
    /// A decoded slot's accessors give back the fields they name.
    ///
    /// `generation()` was pinned; `index()` was read by nothing. Derived
    /// equality compares fields rather than accessors, so
    /// `fn index(self) -> u32 { self.generation }` passed every test in this
    /// crate — and `kernel/src/handle.rs` indexes the arena with it, so every
    /// handle would have resolved to whichever slot the generation happened to
    /// name, with the generation check still passing (D-136).
    #[test]
    fn a_decoded_slot_reports_the_index_and_generation_it_holds() {
        // Distinct, and neither zero: equal values would let one accessor
        // stand in for the other, which is the defect being tested for.
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

    /// Table identities reach a terminal sentinel instead of becoming zero.
    #[test]
    fn table_identity_exhaustion_never_wraps() {
        assert_eq!(next_table_identity(u64::MAX - 1), Some(u64::MAX));
        assert_eq!(next_table_identity(u64::MAX), None);
    }

    /// A tiny set, so the crate needs no dependency to test uniqueness.
    fn alloc_set() -> std::collections::HashSet<u32> {
        std::collections::HashSet::new()
    }
}

#[cfg(test)]
mod rights_tests {
    use super::*;

    /// The universal four and the per-type rights do not share a bit, and
    /// `ALL` is exactly all of them.
    ///
    /// The list is **read out of this file** rather than restated here. A
    /// restated list is a second copy that goes green on the day the two
    /// disagree, which is what happened when the directory rights arrived and
    /// this test was still naming eight (D-161, D-173).
    #[test]
    fn the_two_halves_do_not_overlap() {
        // `pub const NAME: Self = Self(...)`, minus the ones that are unions
        // of the others rather than rights in their own right.
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
            let place: u32 = inside
                .split(" << ")
                .nth(1)
                .expect("a shift")
                .trim()
                .parse()
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

        // And the two halves are disjoint, which is what lets a universal
        // right mean the same thing for every object type.
        assert_eq!(
            Rights::UNIVERSAL.bits() & (seen & !Rights::UNIVERSAL.bits()),
            0
        );
        assert_eq!(Rights::UNIVERSAL.bits().count_ones(), 4);
    }

    /// An undefined bit is refused, not masked away.
    ///
    /// Masking would let a caller ask for a right this kernel does not have
    /// yet and be told yes.
    ///
    /// The undefined bits are found rather than named: `1 << 12` used to be
    /// one and became `TRAVERSE`, which turned this test red — correctly, and
    /// as a reminder that a test naming a *gap* goes stale the moment the gap
    /// is filled (D-173).
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
        // And a word that is mostly right is still refused whole.
        let first_undefined = (0..32u32)
            .find(|bit| defined & (1 << bit) == 0)
            .expect("some bit is undefined");
        assert_eq!(Rights::from_bits(defined | (1 << first_undefined)), None);
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
        assert!(
            Rights::UNIVERSAL.union(memory).are_meaningful_for(memory),
            "its own rights and the universal four"
        );
        assert!(Rights::READ.union(Rights::WAIT).are_meaningful_for(memory));
        // **Not `ALL`.** This used to assert that every right this kernel
        // defines is meaningful for a memory object, which was true only while
        // `ALL` was exactly the universal four plus the memory ones. The
        // directory rights arrived and it became false — correctly, because a
        // memory object has no names in it (D-173).
        assert!(!Rights::ALL.are_meaningful_for(memory));
        assert!(!Rights::TRAVERSE.are_meaningful_for(memory));

        // And a directory, which is the mirror: names but no bytes.
        let directory = Rights::TRAVERSE
            .union(Rights::ENUMERATE)
            .union(Rights::MODIFY);
        assert!(Rights::TRAVERSE.are_meaningful_for(directory));
        assert!(
            !Rights::READ.are_meaningful_for(directory),
            "a directory has no bytes"
        );
        assert!(!Rights::MAP.are_meaningful_for(directory));

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

#[cfg(test)]
mod quota_tests {
    use super::Quota;

    /// Reserving and committing costs the same one slot as taking outright.
    ///
    /// The two paths into a table must agree about what a handle costs, or the
    /// limit means one thing for a handle that arrived by completion and
    /// another for a handle that was inserted.
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

    /// A promise occupies the table before the handle exists.
    ///
    /// The property the whole type is for: an operation admitted against the
    /// last slot must make an ordinary insert fail *now*, not make the
    /// completion fail later. Deleting `reserved` from `charged` leaves every
    /// other test here green.
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

        // And giving the promise back frees it again, which is the other half:
        // a reservation that could not be released would be a slow leak.
        let released = Quota::release(promised).expect("a promise to give back");
        assert_eq!(released, Quota::EMPTY);
        assert!(Quota::take(released, limit).is_some());
    }

    /// Releasing a promise does not disturb the handles already held.
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

    /// Spending what was never promised is refused rather than wrapped.
    ///
    /// Each of these would otherwise borrow from the other count: a `commit`
    /// with nothing reserved subtracts one from the high half and lands a
    /// billion promises in the low half's carry.
    #[test]
    fn spending_a_promise_that_was_not_made_is_refused() {
        assert!(Quota::commit(Quota::EMPTY).is_none());
        assert!(Quota::release(Quota::EMPTY).is_none());
        assert!(Quota::close(Quota::EMPTY).is_none());

        let held = Quota::take(Quota::EMPTY, 4).expect("room");
        assert!(Quota::commit(held).is_none(), "nothing was promised");
        assert!(Quota::release(held).is_none(), "nothing was promised");
        assert_eq!(Quota::close(held), Some(Quota::EMPTY));
    }

    /// The limit check cannot be defeated by two counts that sum past a word.
    ///
    /// `charged` is `u64` for this: `live` and `reserved` are each a `u32`,
    /// and adding them in `u32` wraps to a small number, which passes a limit
    /// check that should have refused. Not a reachable state through this
    /// API — the limit stops it long before — which is exactly why the
    /// arithmetic has to be right on values the API will not produce.
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

    /// The two counts do not read each other's half of the word.
    ///
    /// Pairwise distinct on purpose (D-136): equal counts would let `live` and
    /// `reserved` be swapped with every assertion above still passing.
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
