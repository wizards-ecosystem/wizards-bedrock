use native_abi::Rights;
use world_tree::NodeRef;
use wost::mm::slab::Removal;

use crate::arena::Local;
use crate::channel::{self, Endpoint};
use crate::memory::MemoryObject;
use crate::world::World;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ObjectType {
    Endpoint,
    File,
    Directory,
    Memory,
    World,
}

impl ObjectType {
    pub(crate) const fn per_type_rights(self) -> Rights {
        match self {
            Self::Endpoint => Rights::READ.union(Rights::WRITE),
            Self::File => Rights::READ,
            Self::Directory => Rights::TRAVERSE.union(Rights::ENUMERATE),
            Self::Memory => Rights::READ
                .union(Rights::WRITE)
                .union(Rights::EXECUTE)
                .union(Rights::MAP),
            Self::World => Rights::SPAWN.union(Rights::KILL),
        }
    }
}

pub(crate) const fn file(file: crate::vfs::File) -> ObjectRef {
    ObjectRef::File(file)
}

pub(crate) const fn directory(directory: crate::vfs::Directory) -> ObjectRef {
    ObjectRef::Directory(directory)
}

// An object reference and its rights as plain bytes, so a frame can hold it; zero is none.
pub(crate) const ENCODED: usize = 24;

// A copy names the object; only `acquire` and `release` change what keeps it alive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ObjectRef {
    Endpoint(Local<Endpoint>),
    File(crate::vfs::File),
    Directory(crate::vfs::Directory),
    Memory(Local<MemoryObject>),
    // Weak: a world handle never keeps its world alive, and resolves under a pin or answers `DEAD`.
    World(NodeRef),
}

impl ObjectRef {
    pub(crate) fn kind(&self) -> ObjectType {
        match self {
            Self::Endpoint(_) => ObjectType::Endpoint,
            Self::File(_) => ObjectType::File,
            Self::Directory(_) => ObjectType::Directory,
            Self::Memory(_) => ObjectType::Memory,
            Self::World(_) => ObjectType::World,
        }
    }

    pub(crate) const fn as_file(&self) -> Option<&crate::vfs::File> {
        match self {
            Self::File(file) => Some(file),
            Self::Endpoint(_) | Self::Directory(_) | Self::Memory(_) | Self::World(_) => None,
        }
    }

    pub(crate) const fn as_directory(&self) -> Option<&crate::vfs::Directory> {
        match self {
            Self::Directory(directory) => Some(directory),
            Self::Endpoint(_) | Self::File(_) | Self::Memory(_) | Self::World(_) => None,
        }
    }

    pub(crate) fn encode(self, rights: Rights) -> [u8; ENCODED] {
        let (tag, first, second): (u8, u64, u64) = match self {
            Self::Endpoint(endpoint) => (1, endpoint.to_bits(), 0),
            Self::Memory(memory) => (2, memory.to_bits(), 0),
            Self::World(node) => (3, u64::from(node.node), u64::from(node.generation)),
            Self::File(crate::vfs::File::Fat { cluster, size }) => {
                (4, u64::from(cluster), u64::from(size))
            }
            Self::File(crate::vfs::File::Tmp { at, len }) => (5, at as u64, len as u64),
            Self::Directory(crate::vfs::Directory::Fat { cluster }) => (6, u64::from(cluster), 0),
            Self::Directory(crate::vfs::Directory::Tmp { index }) => (7, index as u64, 0),
            Self::Directory(crate::vfs::Directory::Dev) => (8, 0, 0),
        };
        let mut bytes = [0; ENCODED];
        bytes[0] = tag;
        bytes[4..8].copy_from_slice(&rights.bits().to_le_bytes());
        bytes[8..16].copy_from_slice(&first.to_le_bytes());
        bytes[16..].copy_from_slice(&second.to_le_bytes());
        bytes
    }

    // Only what `encode` wrote is ever decoded, so the narrowing casts are exact.
    pub(crate) fn decode(bytes: [u8; ENCODED]) -> Option<(Self, Rights)> {
        let word = |at: usize| {
            let mut word = [0; 8];
            word.copy_from_slice(&bytes[at..at + 8]);
            u64::from_le_bytes(word)
        };
        let (first, second) = (word(8), word(16));
        let rights =
            Rights::from_bits(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]))?;
        let object = match bytes[0] {
            1 => Self::Endpoint(Local::from_bits(first)),
            2 => Self::Memory(Local::from_bits(first)),
            3 => Self::World(NodeRef {
                node: first as u32,
                generation: second as u32,
            }),
            4 => Self::File(crate::vfs::File::Fat {
                cluster: first as u32,
                size: second as u32,
            }),
            5 => Self::File(crate::vfs::File::Tmp {
                at: first as usize,
                len: second as usize,
            }),
            6 => Self::Directory(crate::vfs::Directory::Fat {
                cluster: first as u32,
            }),
            7 => Self::Directory(crate::vfs::Directory::Tmp {
                index: first as usize,
            }),
            8 => Self::Directory(crate::vfs::Directory::Dev),
            _ => return None,
        };
        Some((object, rights))
    }

    // One more strong reference for a second handle; `false` when the object is already gone.
    pub(crate) fn acquire(self, world: &World) -> bool {
        match self {
            Self::Endpoint(endpoint) => world.endpoints.acquire(endpoint).is_ok(),
            Self::Memory(memory) => world.memories.acquire(memory).is_ok(),
            Self::File(_) | Self::Directory(_) | Self::World(_) => true,
        }
    }

    pub(crate) fn release(self, world: &World) {
        match self {
            Self::Endpoint(endpoint) => channel::release(world, endpoint),
            // Every hold is covered by its holder's strong reference, so the last one meets none.
            Self::Memory(memory) => {
                if let Err(Removal::Borrowed) = world.memories.destroy(memory) {
                    panic!("a memory object's last reference went while a hold borrowed it");
                }
            }
            Self::File(_) | Self::Directory(_) | Self::World(_) => {}
        }
    }
}
