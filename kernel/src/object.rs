//! The things a handle can name.
//!
//! Channels currently provide only the opaque identity used by the bootstrap
//! handle. File and directory references are copyable VFS identities. Message
//! transport waits for a concrete Wizzy caller (D-248).

use alloc::sync::Arc;
#[cfg(feature = "selftest")]
use alloc::sync::Weak;
#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicUsize, Ordering};

use handle_space::Rights;

/// What kind of thing a handle names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum ObjectType {
    /// An opaque bootstrap identity.
    Channel,
    /// A file: something with bytes in it.
    File,
    /// A directory: something with names in it.
    Directory,
}

impl ObjectType {
    /// The rights that mean something for this kind of object.
    pub(crate) const fn per_type_rights(self) -> Rights {
        match self {
            // Bootstrap channels expose identity, not data operations. The
            // universal rights remain meaningful for every object.
            Self::Channel => Rights::NONE,
            Self::File => Rights::READ,
            Self::Directory => Rights::TRAVERSE.union(Rights::ENUMERATE),
        }
    }
}

/// Name a file with an object reference, ready for a handle table.
pub(crate) const fn file(file: crate::vfs::File) -> ObjectRef {
    ObjectRef::File(file)
}

/// Name a directory with an object reference.
pub(crate) const fn directory(directory: crate::vfs::Directory) -> ObjectRef {
    ObjectRef::Directory(directory)
}

/// Something a handle can name.
#[derive(Debug)]
pub(crate) enum ObjectRef {
    /// An opaque bootstrap identity.
    Channel(Arc<Channel>),
    /// A file in some filesystem.
    File(crate::vfs::File),
    /// A directory in some filesystem.
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
    /// What kind of object this is.
    pub(crate) fn kind(&self) -> ObjectType {
        match self {
            Self::Channel(_) => ObjectType::Channel,
            Self::File(_) => ObjectType::File,
            Self::Directory(_) => ObjectType::Directory,
        }
    }

    /// The file inside, if that is what this is.
    pub(crate) const fn as_file(&self) -> Option<&crate::vfs::File> {
        match self {
            Self::File(file) => Some(file),
            Self::Channel(_) | Self::Directory(_) => None,
        }
    }

    /// The directory inside, if that is what this is.
    pub(crate) const fn as_directory(&self) -> Option<&crate::vfs::Directory> {
        match self {
            Self::Directory(directory) => Some(directory),
            Self::Channel(_) | Self::File(_) => None,
        }
    }

    /// A weak reference for lifetime checks on counted objects.
    #[cfg(feature = "selftest")]
    pub(crate) fn downgrade_for_test(&self) -> Option<Weak<Channel>> {
        match self {
            Self::Channel(channel) => Some(Arc::downgrade(channel)),
            Self::File(_) | Self::Directory(_) => None,
        }
    }
}

/// The opaque identity passed to a new program.
#[derive(Debug)]
pub(crate) struct Channel;

#[cfg(feature = "selftest")]
static LIVE_CHANNELS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
impl Drop for Channel {
    fn drop(&mut self) {
        LIVE_CHANNELS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Number of live channel identities, for resource-ledger checks.
#[cfg(feature = "selftest")]
pub(crate) fn live_channels_for_test() -> usize {
    LIVE_CHANNELS.load(Ordering::Acquire)
}

/// Make an opaque bootstrap identity.
pub(crate) fn bootstrap_identity() -> ObjectRef {
    #[cfg(feature = "selftest")]
    LIVE_CHANNELS.fetch_add(1, Ordering::AcqRel);
    ObjectRef::Channel(Arc::new(Channel))
}
