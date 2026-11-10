//! The directory abstraction, and the resolver that cannot ascend.
//!
//! Directories are the abstraction because path resolution is the only code that must be
//! backend-agnostic (`012` §7); dispatch stays an enum until a backend lives in userspace.
//! `libs/path-walk` resolves `..` lexically and refuses underflow, so only descending names
//! arrive, and this file holds no parent pointer at all. The invariant, mutation-tested:
//!
//! > No sequence of path operations from a directory handle can yield a node
//! > that is not a descendant of that handle's node.

use core::fmt;

use fat32::FatError;
use path_walk::PathError;

/// Components a path may have, so the length of a ring 3 argument cannot set the resolver's work.
const MAX_DEPTH: usize = 16;

/// A directory, whichever backend it belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Directory {
    /// A FAT32 directory, named by the cluster its entries begin at: FAT has no inode (`012` §1).
    Fat {
        /// Where its entries begin.
        cluster: u32,
    },
    /// A directory in the filesystem made of memory.
    Tmp {
        /// Its index in the arena.
        index: usize,
    },
    /// The directory of devices, flat because there is nothing to nest.
    Dev,
}

/// A file, whichever backend it belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum File {
    /// A FAT32 file: where its contents begin, and how many bytes there are.
    Fat {
        /// The first cluster of its contents.
        cluster: u32,
        /// Bytes in it.
        size: u32,
    },
    /// A file in memory: where its bytes are in the arena, and how many.
    Tmp {
        /// Where its bytes begin.
        at: usize,
        /// How many there are.
        len: usize,
    },
}

/// What a name resolved to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Node {
    /// Something with names in it.
    Directory(Directory),
    /// Something with bytes in it.
    File(File),
    /// A device.
    Device(crate::devfs::Device),
}

/// Why a path did not resolve.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VfsError {
    /// The path itself is not one this kernel can resolve, and which mistake it makes.
    Path(PathError),
    /// The backend refused.
    Backend(FatError),
    /// There is no volume to resolve against.
    NoVolume,
    /// A component that must be a directory is not.
    NotADirectory,
    /// The name is not there.
    NotFound,
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(reason) => write!(f, "path: {reason}"),
            Self::Backend(reason) => write!(f, "backend: {reason}"),
            Self::NoVolume => f.write_str("no volume to resolve against"),
            Self::NotADirectory => f.write_str("a component is not a directory"),
            Self::NotFound => f.write_str("the name is not there"),
        }
    }
}

impl core::error::Error for VfsError {}

impl From<PathError> for VfsError {
    fn from(error: PathError) -> Self {
        Self::Path(error)
    }
}

impl From<FatError> for VfsError {
    fn from(error: FatError) -> Self {
        match error {
            FatError::NotFound => Self::NotFound,
            FatError::WrongKind => Self::NotADirectory,
            other => Self::Backend(other),
        }
    }
}

/// The directory a program starts from: one namespace per program, with no way to compose two
/// (a deliberate hole, `014`), so a program has only the directory handles it was given.
pub(crate) fn root() -> Option<Directory> {
    crate::fs::mount()
        .and_then(Result::ok)
        .map(|volume| Directory::Fat {
            cluster: volume.root_cluster(),
        })
}

/// The root of the filesystem made of memory.
pub(crate) const fn tmp_root() -> Directory {
    Directory::Tmp {
        index: crate::tmpfs::ROOT,
    }
}

/// The directory of devices.
pub(crate) const fn dev_root() -> Directory {
    Directory::Dev
}

/// Resolve `path` from `at` inside one mount, node to node, so nothing is looked up twice and no
/// component changes between being judged a directory and used (`openat2`'s gap).
/// Absolute and escaping paths are malformed `Path` arguments, since no root or parent exists.
pub(crate) fn resolve(at: Directory, path: &[u8]) -> Result<Node, VfsError> {
    let mut components = [b"".as_slice(); MAX_DEPTH];
    let depth = path_walk::split(path, &mut components)?;

    // Nothing to walk: `.` and `a/..` name the directory the walk started from.
    if depth == 0 {
        return Ok(Node::Directory(at));
    }

    match at {
        Directory::Fat { cluster } => resolve_in_fat(cluster, &components[..depth]),
        Directory::Tmp { index } => resolve_in_tmp(index, &components[..depth]),
        Directory::Dev => resolve_in_dev(&components[..depth]),
    }
}

/// Walk `components` from a directory in memory.
fn resolve_in_tmp(from: usize, components: &[&[u8]]) -> Result<Node, VfsError> {
    let mut directory = from;

    for (index, name) in components.iter().enumerate() {
        let found = crate::tmpfs::lookup(directory, name).ok_or(VfsError::NotFound)?;
        let is_last = index + 1 == components.len();

        match found {
            crate::tmpfs::Found::Directory(next) => {
                if is_last {
                    return Ok(Node::Directory(Directory::Tmp { index: next }));
                }
                directory = next;
            }
            crate::tmpfs::Found::File { at, len } => {
                if !is_last {
                    // A file walked through would be answered from whatever its bytes say.
                    return Err(VfsError::NotADirectory);
                }
                return Ok(Node::File(File::Tmp { at, len }));
            }
        }
    }

    Err(VfsError::NotFound)
}

/// Resolve a name among the devices: one component only, so `storage/anything` is refused as a
/// file walked through, the same as in every backend.
fn resolve_in_dev(components: &[&[u8]]) -> Result<Node, VfsError> {
    let (first, rest) = components.split_first().ok_or(VfsError::NotFound)?;
    let device = crate::devfs::lookup(first).ok_or(VfsError::NotFound)?;
    if rest.is_empty() {
        Ok(Node::Device(device))
    } else {
        Err(VfsError::NotADirectory)
    }
}

/// Walk `components` from a FAT32 directory.
fn resolve_in_fat(from: u32, components: &[&[u8]]) -> Result<Node, VfsError> {
    let mut volume = crate::fs::mount()
        .ok_or(VfsError::NoVolume)?
        .map_err(VfsError::from)?;
    let entry = volume.resolve(from, components).map_err(VfsError::from)?;
    Ok(if entry.is_directory() {
        Node::Directory(Directory::Fat {
            cluster: entry.cluster,
        })
    } else {
        Node::File(File::Fat {
            cluster: entry.cluster,
            size: entry.size,
        })
    })
}

/// Read `into.len()` bytes of `file` from `offset`; offsets are per operation (`012` §6), since a
/// shared cursor would be a hot-path lock and a covert channel between two handles to one file.
pub(crate) fn read(file: File, offset: u32, into: &mut [u8]) -> Result<usize, VfsError> {
    match file {
        File::Fat { cluster, size } => crate::fs::mount()
            .ok_or(VfsError::NoVolume)?
            .map_err(VfsError::from)?
            .read_file(cluster, size, offset, into)
            .map_err(VfsError::from),
        File::Tmp { at, len } => Ok(crate::tmpfs::read(at, len, offset as usize, into)),
    }
}

/// Call `found` for every name in `directory`, stopping when it returns false.
pub(crate) fn enumerate(
    directory: Directory,
    mut found: impl FnMut(&[u8], bool) -> bool,
) -> Result<(), VfsError> {
    match directory {
        Directory::Fat { cluster } => {
            let mut volume = crate::fs::mount()
                .ok_or(VfsError::NoVolume)?
                .map_err(VfsError::from)?;
            let mut cursor = fat32::DirectoryCursor::new(cluster);
            loop {
                let chunk = volume
                    .directory_chunk(&mut cursor)
                    .map_err(VfsError::from)?;
                for entry in chunk.entries() {
                    if !found(entry.name(), entry.is_directory()) {
                        return Ok(());
                    }
                }
                if cursor.is_done() {
                    return Ok(());
                }
            }
        }
        Directory::Tmp { index } => {
            let listing = crate::tmpfs::list(index).ok_or(VfsError::NotFound)?;
            for entry in listing.entries() {
                if !found(entry.name(), entry.is_directory()) {
                    break;
                }
            }
            Ok(())
        }
        Directory::Dev => {
            crate::devfs::walk(found);
            Ok(())
        }
    }
}
