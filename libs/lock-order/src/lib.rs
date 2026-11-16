#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod proof;
// Type-level ranks that rustc checks; Verus cannot represent the uninhabited `Unlocked`.
#[cfg_attr(verus_only, verifier::external)]
mod rank;

#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{verus_spec, verus_verify};

pub use rank::{Level, LockAfter, LockBefore, Unlocked};

#[verus_verify]
pub const LEAF: u8 = 63;

// Each level names one immediate predecessor, a root names `Unlocked` by that bare name.
#[macro_export]
macro_rules! impl_lock_after {
    (Unlocked => $after:ty) => {
        const _: () = assert!(
            0 < <$after as $crate::Level>::RANK && <$after as $crate::Level>::RANK <= $crate::LEAF,
            "a level ranks above zero and at most at the leaf rank"
        );
        impl $crate::LockAfter<$crate::Unlocked> for $after {}
    };
    ($before:ty => $after:ty) => {
        const _: () = assert!(
            <$before as $crate::Level>::RANK < <$after as $crate::Level>::RANK
                && <$after as $crate::Level>::RANK <= $crate::LEAF,
            "a level ranks above its predecessor and at most at the leaf rank"
        );
        impl $crate::LockAfter<$before> for $after {}
        impl<X: $crate::LockBefore<$before>> $crate::LockAfter<X> for $after {}
    };
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct HeldRanks(u64);

#[verus_verify]
impl HeldRanks {
    #[verus_spec(r =>
        ensures
            r.spec_bits() == 0,
    )]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[verus_spec(r =>
        ensures
            r.spec_bits() == bits,
    )]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_bits(),
    )]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[must_use]
    #[verus_spec(r =>
        ensures
            match r {
                Some(held) => proof::taken(self.spec_bits(), rank) == Some(held.spec_bits()),
                None => proof::taken(self.spec_bits(), rank) is None,
            },
    )]
    pub const fn take(self, rank: u8) -> Option<Self> {
        if rank == 0 || rank > LEAF {
            return None;
        }
        if self.0 & (u64::MAX << rank) != 0 {
            return None;
        }
        Some(Self(self.0 | 1 << rank))
    }

    #[must_use]
    #[verus_spec(r =>
        ensures
            match r {
                Some(held) => proof::released(self.spec_bits(), rank) == Some(held.spec_bits()),
                None => proof::released(self.spec_bits(), rank) is None,
            },
    )]
    pub const fn release(self, rank: u8) -> Option<Self> {
        if rank == 0 || rank > LEAF {
            return None;
        }
        let bit = 1 << rank;
        if self.0 & bit == 0 {
            return None;
        }
        Some(Self(self.0 & !bit))
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    node: u32,
    index: u32,
}

#[verus_verify]
impl Key {
    #[verus_spec(r =>
        ensures
            r.spec_node() == node && r.spec_index() == index,
    )]
    pub const fn new(node: u32, index: u32) -> Self {
        Self { node, index }
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_node(),
    )]
    pub const fn node(self) -> u32 {
        self.node
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_index(),
    )]
    pub const fn index(self) -> u32 {
        self.index
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pair {
    AsGiven,
    Swapped,
}

#[must_use]
#[verus_spec(r =>
    ensures
        r == proof::ascending_by_object(first, second),
)]
pub const fn pair(first: Key, second: Key) -> Option<Pair> {
    if first.node < second.node {
        return Some(Pair::AsGiven);
    }
    if first.node > second.node {
        return Some(Pair::Swapped);
    }
    if first.index < second.index {
        return Some(Pair::AsGiven);
    }
    if first.index > second.index {
        return Some(Pair::Swapped);
    }
    None
}
