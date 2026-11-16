use native_abi::Rights;
use wost::mm::slab::Removal;

use crate::arena::Local;
use crate::memory::MemoryObject;
use crate::world::World;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ObjectType {
    Channel,
    File,
    Directory,
    Memory,
}

impl ObjectType {
    pub(crate) const fn per_type_rights(self) -> Rights {
        match self {
            Self::Channel => Rights::NONE,
            Self::File => Rights::READ,
            Self::Directory => Rights::TRAVERSE.union(Rights::ENUMERATE),
            Self::Memory => Rights::READ
                .union(Rights::WRITE)
                .union(Rights::EXECUTE)
                .union(Rights::MAP),
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
    Channel(Local<Channel>),
    File(crate::vfs::File),
    Directory(crate::vfs::Directory),
    Memory(Local<MemoryObject>),
}

impl ObjectRef {
    pub(crate) fn kind(&self) -> ObjectType {
        match self {
            Self::Channel(_) => ObjectType::Channel,
            Self::File(_) => ObjectType::File,
            Self::Directory(_) => ObjectType::Directory,
            Self::Memory(_) => ObjectType::Memory,
        }
    }

    pub(crate) const fn as_file(&self) -> Option<&crate::vfs::File> {
        match self {
            Self::File(file) => Some(file),
            Self::Channel(_) | Self::Directory(_) | Self::Memory(_) => None,
        }
    }

    pub(crate) const fn as_directory(&self) -> Option<&crate::vfs::Directory> {
        match self {
            Self::Directory(directory) => Some(directory),
            Self::Channel(_) | Self::File(_) | Self::Memory(_) => None,
        }
    }

    // One more strong reference for a second handle; `false` when the object is already gone.
    pub(crate) fn acquire(self, world: &World) -> bool {
        match self {
            Self::Channel(channel) => world.channels.acquire(channel).is_ok(),
            Self::Memory(memory) => world.memories.acquire(memory).is_ok(),
            Self::File(_) | Self::Directory(_) => true,
        }
    }

    pub(crate) fn release(self, world: &World) {
        match self {
            Self::Channel(channel) => {
                let _ = world.channels.destroy(channel);
            }
            // Every hold is covered by its holder's strong reference, so the last one meets none.
            Self::Memory(memory) => {
                if let Err(Removal::Borrowed) = world.memories.destroy(memory) {
                    panic!("a memory object's last reference went while a hold borrowed it");
                }
            }
            Self::File(_) | Self::Directory(_) => {}
        }
    }
}

#[derive(Debug)]
pub(crate) struct Channel;

// The bootstrap identity: a channel object in the world, held by the table's first entry.
pub(crate) fn bootstrap_identity(
    earmark: &mut wost::mm::pool::Earmark,
    world: &World,
) -> ObjectRef {
    ObjectRef::Channel(world.channels.insert(earmark, Channel))
}
