#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub const LEAF: u8 = 63;

pub trait Level {
    const RANK: u8;
}

pub trait LockAfter<A> {}

pub trait LockBefore<B> {}

impl<B: LockAfter<A>, A> LockBefore<B> for A {}

#[derive(Debug)]
pub enum Unlocked {}

impl Level for Unlocked {
    const RANK: u8 = 0;
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct HeldRanks(u64);

impl HeldRanks {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    #[must_use]
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    node: u32,
    index: u32,
}

impl Key {
    pub const fn new(node: u32, index: u32) -> Self {
        Self { node, index }
    }

    pub const fn node(self) -> u32 {
        self.node
    }

    pub const fn index(self) -> u32 {
        self.index
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pair {
    AsGiven,
    Swapped,
}

#[must_use]
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
