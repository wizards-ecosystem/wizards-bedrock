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
