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

#[verus_verify]
const LINK_SHIFT: u64 = 6;

#[verus_verify]
const LINK_MASK: u64 = (LINKED_FRAMES as u64) << LINK_SHIFT;

// Frames numbered below this fit a link, which is biased by one in bits 6 to 31.
#[verus_verify]
pub const LINKED_FRAMES: u32 = 0x3ff_ffff;

mod owner {
    use vstd::prelude::verus;

    verus! {

    pub(super) const FREE: u64 = 0;
    pub(super) const KERNEL: u64 = 1;
    pub(super) const TABLE: u64 = 2;
    pub(super) const DEVICE: u64 = 3;
    pub(super) const ARENA: u64 = 4;
    pub(super) const SURRENDERED: u64 = 5;

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

// The next frame on an arena's surrendered list, or its end.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Link(pub(crate) u32);

#[verus_verify]
impl Link {
    #[verus_spec(r =>
        ensures
            r matches Some(link) ==> link.spec_get() as int == frame as int + 1
                && link.spec_get() <= 0x3ff_ffff,
            r.is_none() == (frame >= 0x3ff_ffff),
    )]
    pub fn to(frame: usize) -> Option<Self> {
        proof! {
            assert(LINKED_FRAMES == 0x3ff_ffffu32) by (compute_only);
        }
        if frame >= LINKED_FRAMES as usize {
            return None;
        }
        Some(Self(frame as u32 + 1))
    }

    #[verus_spec(r =>
        ensures
            r matches Some(frame) ==> frame as int + 1 == self.spec_get() as int,
            r.is_none() == (self.spec_get() == 0),
    )]
    pub fn frame(self) -> Option<usize> {
        if self.0 == 0 {
            return None;
        }
        Some((self.0 - 1) as usize)
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
    Surrendered(ArenaIndex, FrameKind, Link),
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

    // A surrendered frame stays its arena's until dissolve, and its record links the next one.
    #[verus_spec(r =>
        ensures
            r matches Some(record) ==> proof::surrenders(self, arena, next) && record.spec_owner()
                == proof::surrender_of(self.spec_owner(), next) && record.spec_access()
                == Access::Writable,
            r.is_none() ==> !proof::surrenders(self, arena, next),
    )]
    pub fn surrendered(self, arena: ArenaIndex, next: Link) -> Option<Self> {
        proof! {
            assert(LINKED_FRAMES == 0x3ff_ffffu32) by (compute_only);
        }
        if next.0 > LINKED_FRAMES || matches!(self.access, Access::Sealed) {
            return None;
        }
        match self.owner {
            Owner::Arena(index, kind) => {
                if index.0 == arena.0 {
                    Some(Self {
                        owner: Owner::Surrendered(index, kind, next),
                        access: Access::Writable,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    #[verus_spec(r =>
        ensures
            r == proof::encoded(self),
    )]
    pub fn encode(self) -> u64 {
        proof! {
            assert(OWNER_MASK == 7u64 && SEALED == 8u64 && KIND_MASK == 0x30u64 && KIND_SHIFT == 4u64
                && ARENA_SHIFT == 32u64 && ARENA_BITS == 0xffff_ffff_0000_0000u64 && LINK_SHIFT
                == 6u64) by (compute_only);
            assert(owner::FREE == 0u64 && owner::KERNEL == 1u64 && owner::TABLE == 2u64
                && owner::DEVICE == 3u64 && owner::ARENA == 4u64 && owner::SURRENDERED == 5u64);
        }
        let owner = match self.owner {
            Owner::Free => owner::FREE,
            Owner::Kernel => owner::KERNEL,
            Owner::Table => owner::TABLE,
            Owner::Device => owner::DEVICE,
            Owner::Arena(arena, frame) => {
                owner::ARENA | kind_word(frame) << KIND_SHIFT | u64::from(arena.0) << ARENA_SHIFT
            }
            Owner::Surrendered(arena, frame, next) => {
                owner::SURRENDERED
                    | kind_word(frame) << KIND_SHIFT
                    | u64::from(next.0) << LINK_SHIFT
                    | u64::from(arena.0) << ARENA_SHIFT
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
                && ARENA_SHIFT == 32u64 && ARENA_BITS == 0xffff_ffff_0000_0000u64 && LINK_SHIFT
                == 6u64 && LINK_MASK == 0xffff_ffc0u64) by (compute_only);
            assert(owner::FREE == 0u64 && owner::KERNEL == 1u64 && owner::TABLE == 2u64
                && owner::DEVICE == 3u64 && owner::ARENA == 4u64 && owner::SURRENDERED == 5u64);
            assert((word & 0xffff_ffc0u64) >> 6u64 <= 0x3ff_ffffu64) by (bit_vector);
        }
        let class = word & OWNER_MASK;
        if (class != owner::SURRENDERED && word & LINK_MASK != 0)
            || (class < owner::ARENA && word & (KIND_MASK | ARENA_BITS) != 0)
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
            owner::ARENA => match kind_of(word) {
                Some(frame) => Owner::Arena(ArenaIndex((word >> ARENA_SHIFT) as u32), frame),
                None => {
                    proof! {
                        proof::lemma_no_encoding(word);
                    }
                    return None;
                }
            },
            owner::SURRENDERED => match kind_of(word) {
                Some(frame) => Owner::Surrendered(
                    ArenaIndex((word >> ARENA_SHIFT) as u32),
                    frame,
                    Link(((word & LINK_MASK) >> LINK_SHIFT) as u32),
                ),
                None => {
                    proof! {
                        proof::lemma_no_encoding(word);
                    }
                    return None;
                }
            },
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
        r == proof::kind_bits(frame),
)]
fn kind_word(frame: FrameKind) -> u64 {
    proof! {
        assert(kind::OBJECT == 0u64 && kind::DATA == 1u64 && kind::TABLE == 2u64);
    }
    match frame {
        FrameKind::ArenaObject => kind::OBJECT,
        FrameKind::ArenaData => kind::DATA,
        FrameKind::Table => kind::TABLE,
    }
}

#[verus_spec(r =>
    ensures
        r matches Some(frame) ==> proof::kind_bits(frame) == (word & 0x30u64) >> 4u64,
        r.is_none() ==> (word & 0x30u64) >> 4u64 == 3,
)]
fn kind_of(word: u64) -> Option<FrameKind> {
    proof! {
        assert(KIND_MASK == 0x30u64 && KIND_SHIFT == 4u64) by (compute_only);
        assert(kind::OBJECT == 0u64 && kind::DATA == 1u64 && kind::TABLE == 2u64);
        assert((word & 0x30u64) >> 4u64 <= 3) by (bit_vector);
    }
    match (word & KIND_MASK) >> KIND_SHIFT {
        kind::OBJECT => Some(FrameKind::ArenaObject),
        kind::DATA => Some(FrameKind::ArenaData),
        kind::TABLE => Some(FrameKind::Table),
        _ => None,
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

impl Link {
    pub exec const END: Self
        ensures
            Self::END.spec_get() == 0,
    {
        Self(0)
    }

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
            (Owner::Surrendered(a, k, l), Owner::Surrendered(b, j, m)) => {
                assert(a.0 as u64 == b.0 as u64);
                assert(proof::kind_bits(k) == proof::kind_bits(j));
                assert(l.0 as u64 == m.0 as u64);
            },
            _ => {},
        }
    }
}

} // verus!
