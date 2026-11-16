use alloc::sync::Arc;

use native_abi::Rights;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ObjectType {
    Channel,
    File,
    Directory,
}

impl ObjectType {
    pub(crate) const fn per_type_rights(self) -> Rights {
        match self {
            Self::Channel => Rights::NONE,
            Self::File => Rights::READ,
            Self::Directory => Rights::TRAVERSE.union(Rights::ENUMERATE),
        }
    }
}

pub(crate) const fn file(file: crate::vfs::File) -> ObjectRef {
    ObjectRef::File(file)
}

pub(crate) const fn directory(directory: crate::vfs::Directory) -> ObjectRef {
    ObjectRef::Directory(directory)
}

#[derive(Debug)]
pub(crate) enum ObjectRef {
    Channel(Arc<Channel>),
    File(crate::vfs::File),
    Directory(crate::vfs::Directory),
}

impl Clone for ObjectRef {
    fn clone(&self) -> Self {
        match self {
            Self::Channel(channel) => Self::Channel(Arc::clone(channel)),
            Self::File(file) => Self::File(*file),
            Self::Directory(directory) => Self::Directory(*directory),
        }
    }
}

impl ObjectRef {
    pub(crate) fn kind(&self) -> ObjectType {
        match self {
            Self::Channel(_) => ObjectType::Channel,
            Self::File(_) => ObjectType::File,
            Self::Directory(_) => ObjectType::Directory,
        }
    }

    pub(crate) const fn as_file(&self) -> Option<&crate::vfs::File> {
        match self {
            Self::File(file) => Some(file),
            Self::Channel(_) | Self::Directory(_) => None,
        }
    }

    pub(crate) const fn as_directory(&self) -> Option<&crate::vfs::Directory> {
        match self {
            Self::Directory(directory) => Some(directory),
            Self::Channel(_) | Self::File(_) => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Channel;

pub(crate) fn bootstrap_identity() -> ObjectRef {
    ObjectRef::Channel(Arc::new(Channel))
}
