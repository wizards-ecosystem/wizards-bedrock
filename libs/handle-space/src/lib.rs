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
    #[must_use]
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
mod tests;
