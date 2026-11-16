#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{verus, verus_spec, verus_verify};

use crate::proof;

#[verus_verify]
const OWNER_MASK: u64 = 0x7;

#[verus_verify]
const SEALED: u64 = 1 << 3;

#[verus_verify]
const KIND_SHIFT: u64 = 4;

#[verus_verify]
const KIND_MASK: u64 = 0x3 << KIND_SHIFT;

#[verus_verify]
const ARENA_SHIFT: u64 = 32;

#[verus_verify]
const ARENA_BITS: u64 = u64::MAX << ARENA_SHIFT;

mod owner {
    use vstd::prelude::verus;

    verus! {

    pub(super) const FREE: u64 = 0;
    pub(super) const KERNEL: u64 = 1;
    pub(super) const TABLE: u64 = 2;
    pub(super) const DEVICE: u64 = 3;
    pub(super) const ARENA: u64 = 4;

    } // verus!
}

mod kind {
    use vstd::prelude::verus;

    verus! {

    pub(super) const OBJECT: u64 = 0;
    pub(super) const DATA: u64 = 1;
    pub(super) const TABLE: u64 = 2;

    } // verus!
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArenaIndex(pub(crate) u32);

#[verus_verify]
impl ArenaIndex {
    #[verus_spec(r =>
        ensures
            r.spec_get() == index,
    )]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_get(),
    )]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameKind {
    ArenaObject,
    ArenaData,
    Table,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Free,
    Kernel,
    Table,
    Device,
    Arena(ArenaIndex, FrameKind),
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Writable,
    Sealed,
}

// Only kernel frames and arena object or data frames seal, a free frame is always writable, and
// zero encodes `FREE`.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record {
    pub(crate) owner: Owner,
    pub(crate) access: Access,
}

#[verus_verify]
impl Record {
    #[verus_spec(r =>
        ensures
            r == self.spec_owner(),
    )]
    pub const fn owner(self) -> Owner {
        self.owner
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_access(),
    )]
    pub const fn access(self) -> Access {
        self.access
    }

    #[verus_spec(r =>
        ensures
            r matches Some(record) ==> self.spec_owner() == Owner::Free && record.spec_owner()
                == owner && record.spec_access() == Access::Writable,
            r.is_none() ==> self.spec_owner() != Owner::Free || owner == Owner::Free,
    )]
    pub const fn allocated(self, owner: Owner) -> Option<Self> {
        if !matches!(self.owner, Owner::Free) || matches!(owner, Owner::Free) {
            return None;
        }
        Some(Self {
            owner,
            access: Access::Writable,
        })
    }

    #[verus_spec(r =>
        ensures
            r matches Some(record) ==> proof::can_seal(self.spec_owner()) && self.spec_access()
                == Access::Writable && record.spec_owner() == self.spec_owner()
                && record.spec_access() == Access::Sealed,
            r.is_none() ==> !proof::can_seal(self.spec_owner()) || self.spec_access()
                == Access::Sealed,
    )]
    pub const fn sealed(self) -> Option<Self> {
        if !can_seal(self.owner) || matches!(self.access, Access::Sealed) {
            return None;
        }
        Some(Self {
            owner: self.owner,
            access: Access::Sealed,
        })
    }

    #[verus_spec(r =>
        ensures
            r matches Some(record) ==> self.spec_owner() != Owner::Free && record.spec_owner()
                == Owner::Free && record.spec_access() == Access::Writable,
            r.is_none() ==> self.spec_owner() == Owner::Free,
    )]
    pub const fn released(self) -> Option<Self> {
        match self.owner {
            Owner::Free => None,
            _ => Some(Self::FREE),
        }
    }

    #[verus_spec(r =>
        ensures
            r == proof::encoded(self),
    )]
    pub fn encode(self) -> u64 {
        proof! {
            assert(OWNER_MASK == 7u64 && SEALED == 8u64 && KIND_MASK == 0x30u64 && KIND_SHIFT == 4u64
                && ARENA_SHIFT == 32u64 && ARENA_BITS == 0xffff_ffff_0000_0000u64) by (compute_only);
            assert(owner::FREE == 0u64 && owner::KERNEL == 1u64 && owner::TABLE == 2u64
                && owner::DEVICE == 3u64 && owner::ARENA == 4u64 && kind::OBJECT == 0u64
                && kind::DATA == 1u64 && kind::TABLE == 2u64);
        }
        let owner = match self.owner {
            Owner::Free => owner::FREE,
            Owner::Kernel => owner::KERNEL,
            Owner::Table => owner::TABLE,
            Owner::Device => owner::DEVICE,
            Owner::Arena(arena, frame) => {
                let frame = match frame {
                    FrameKind::ArenaObject => kind::OBJECT,
                    FrameKind::ArenaData => kind::DATA,
                    FrameKind::Table => kind::TABLE,
                };
                owner::ARENA | frame << KIND_SHIFT | u64::from(arena.0) << ARENA_SHIFT
            }
        };
        match self.access {
            Access::Writable => owner,
            Access::Sealed => owner | SEALED,
        }
    }

    #[verus_spec(r =>
        ensures
            r matches Some(record) ==> proof::encoded(record) == word && proof::well_formed(record),
            r.is_none() ==> forall|record: Self|
                #[trigger] proof::encoded(record) == word ==> !proof::well_formed(record),
    )]
    pub fn decode(word: u64) -> Option<Self> {
        proof! {
            assert(OWNER_MASK == 7u64 && SEALED == 8u64 && KIND_MASK == 0x30u64 && KIND_SHIFT == 4u64
                && ARENA_SHIFT == 32u64 && ARENA_BITS == 0xffff_ffff_0000_0000u64) by (compute_only);
            assert(owner::FREE == 0u64 && owner::KERNEL == 1u64 && owner::TABLE == 2u64
                && owner::DEVICE == 3u64 && owner::ARENA == 4u64 && kind::OBJECT == 0u64
                && kind::DATA == 1u64 && kind::TABLE == 2u64);
        }
        let class = word & OWNER_MASK;
        if word & !(OWNER_MASK | SEALED | KIND_MASK | ARENA_BITS) != 0
            || (class != owner::ARENA && word & (KIND_MASK | ARENA_BITS) != 0)
        {
            proof! {
                proof::lemma_no_encoding(word);
            }
            return None;
        }
        let owner = match class {
            owner::FREE => Owner::Free,
            owner::KERNEL => Owner::Kernel,
            owner::TABLE => Owner::Table,
            owner::DEVICE => Owner::Device,
            owner::ARENA => {
                let frame = match (word & KIND_MASK) >> KIND_SHIFT {
                    kind::OBJECT => FrameKind::ArenaObject,
                    kind::DATA => FrameKind::ArenaData,
                    kind::TABLE => FrameKind::Table,
                    _ => {
                        proof! {
                            assert((word & 0x30u64) >> 4u64 <= 3) by (bit_vector);
                            proof::lemma_no_encoding(word);
                        }
                        return None;
                    }
                };
                Owner::Arena(ArenaIndex((word >> ARENA_SHIFT) as u32), frame)
            }
            _ => {
                proof! {
                    assert(word & 7u64 <= 7) by (bit_vector);
                    proof::lemma_no_encoding(word);
                }
                return None;
            }
        };
        let access = if word & SEALED == 0 {
            Access::Writable
        } else {
            Access::Sealed
        };
        proof! {
            proof::lemma_layout_unique(word, Self { owner, access });
        }
        if matches!(access, Access::Writable) || can_seal(owner) {
            Some(Self { owner, access })
        } else {
            None
        }
    }
}

#[verus_spec(r =>
    ensures
        r == proof::can_seal(owner),
)]
const fn can_seal(owner: Owner) -> bool {
    matches!(
        owner,
        Owner::Kernel | Owner::Arena(_, FrameKind::ArenaObject | FrameKind::ArenaData)
    )
}

verus! {

impl ArenaIndex {
    pub open(crate) spec fn spec_get(self) -> u32 {
        self.0
    }
}

impl Record {
    pub exec const FREE: Self
        ensures
            Self::FREE.spec_owner() == Owner::Free,
            Self::FREE.spec_access() == Access::Writable,
    {
        Self { owner: Owner::Free, access: Access::Writable }
    }

    pub open(crate) spec fn spec_owner(self) -> Owner {
        self.owner
    }

    pub open(crate) spec fn spec_access(self) -> Access {
        self.access
    }

    pub proof fn lemma_same_layout(self, other: Self, word: u64)
        requires
            proof::layout_matches(word, self),
            proof::layout_matches(word, other),
        ensures
            other == self,
    {
        match (self.owner, other.owner) {
            (Owner::Arena(a, k), Owner::Arena(b, j)) => {
                assert(a.0 as u64 == b.0 as u64);
                assert(proof::kind_bits(k) == proof::kind_bits(j));
            },
            _ => {},
        }
    }
}

} // verus!
