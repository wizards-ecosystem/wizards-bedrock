const OWNER_MASK: u64 = 0x7;

const SEALED: u64 = 1 << 3;

const ARENA_SHIFT: u32 = 32;

const ARENA_BITS: u64 = u64::MAX << ARENA_SHIFT;

mod owner {
    pub(super) const FREE: u64 = 0;
    pub(super) const KERNEL: u64 = 1;
    pub(super) const TABLE: u64 = 2;
    pub(super) const DEVICE: u64 = 3;
    pub(super) const ARENA: u64 = 4;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArenaId(u32);

impl ArenaId {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Free,
    Kernel,
    Table,
    Device,
    Arena(ArenaId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Writable,
    Sealed,
}

// Only kernel and arena frames seal, and a free frame is always writable; zero encodes `FREE`.
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
            Owner::Arena(arena) => owner::ARENA | u64::from(arena.0) << ARENA_SHIFT,
        };
        match self.access {
            Access::Writable => owner,
            Access::Sealed => owner | SEALED,
        }
    }

    pub fn decode(word: u64) -> Option<Self> {
        let kind = word & OWNER_MASK;
        let arena = (word >> ARENA_SHIFT) as u32;
        if word & !(OWNER_MASK | SEALED | ARENA_BITS) != 0 || (kind != owner::ARENA && arena != 0) {
            return None;
        }
        let owner = match kind {
            owner::FREE => Owner::Free,
            owner::KERNEL => Owner::Kernel,
            owner::TABLE => Owner::Table,
            owner::DEVICE => Owner::Device,
            owner::ARENA => Owner::Arena(ArenaId(arena)),
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
    matches!(owner, Owner::Kernel | Owner::Arena(_))
}
