const OWNER_MASK: u64 = 0x7;

const SEALED: u64 = 1 << 3;

const KIND_SHIFT: u32 = 4;

const KIND_MASK: u64 = 0x3 << KIND_SHIFT;

const ARENA_SHIFT: u32 = 32;

const ARENA_BITS: u64 = u64::MAX << ARENA_SHIFT;

mod owner {
    pub(super) const FREE: u64 = 0;
    pub(super) const KERNEL: u64 = 1;
    pub(super) const TABLE: u64 = 2;
    pub(super) const DEVICE: u64 = 3;
    pub(super) const ARENA: u64 = 4;
}

mod kind {
    pub(super) const OBJECT: u64 = 0;
    pub(super) const DATA: u64 = 1;
    pub(super) const TABLE: u64 = 2;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArenaIndex(u32);

impl ArenaIndex {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameKind {
    ArenaObject,
    ArenaData,
    Table,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Free,
    Kernel,
    Table,
    Device,
    Arena(ArenaIndex, FrameKind),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Writable,
    Sealed,
}

// Only kernel frames and arena object or data frames seal, a free frame is always writable, and
// zero encodes `FREE`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record {
    owner: Owner,
    access: Access,
}

impl Record {
    pub const FREE: Self = Self {
        owner: Owner::Free,
        access: Access::Writable,
    };

    pub const fn owner(self) -> Owner {
        self.owner
    }

    pub const fn access(self) -> Access {
        self.access
    }

    pub const fn allocated(self, owner: Owner) -> Option<Self> {
        if !matches!(self.owner, Owner::Free) || matches!(owner, Owner::Free) {
            return None;
        }
        Some(Self {
            owner,
            access: Access::Writable,
        })
    }

    pub const fn sealed(self) -> Option<Self> {
        if !can_seal(self.owner) || matches!(self.access, Access::Sealed) {
            return None;
        }
        Some(Self {
            owner: self.owner,
            access: Access::Sealed,
        })
    }

    pub const fn released(self) -> Option<Self> {
        match self.owner {
            Owner::Free => None,
            _ => Some(Self::FREE),
        }
    }

    pub fn encode(self) -> u64 {
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

    pub fn decode(word: u64) -> Option<Self> {
        let class = word & OWNER_MASK;
        if word & !(OWNER_MASK | SEALED | KIND_MASK | ARENA_BITS) != 0
            || (class != owner::ARENA && word & (KIND_MASK | ARENA_BITS) != 0)
        {
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
                    _ => return None,
                };
                Owner::Arena(ArenaIndex((word >> ARENA_SHIFT) as u32), frame)
            }
            _ => return None,
        };
        let access = if word & SEALED == 0 {
            Access::Writable
        } else {
            Access::Sealed
        };
        (access == Access::Writable || can_seal(owner)).then_some(Self { owner, access })
    }
}

const fn can_seal(owner: Owner) -> bool {
    matches!(
        owner,
        Owner::Kernel | Owner::Arena(_, FrameKind::ArenaObject | FrameKind::ArenaData)
    )
}
