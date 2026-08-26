//! The directory abstraction, and the resolver that cannot ascend.
//!
//! **Not Linux's shape** (`012` §7). Its five operation tables and the dcache
//! exist to serve a global namespace with a mount tree and a shared lookup
//! cache, and this kernel has neither. Importing the shape without the
//! requirements is how a VFS becomes premature.
//!
//! **The abstraction is over directories**, because path resolution is the only
//! code that must be backend-agnostic: `lookup`, `enumerate`, and — when there
//! is a write path — `modify`. That is 9P's walk/read/remove, and it is three
//! operations rather than five tables.
//!
//! **Enum dispatch while every backend is in-kernel.** Monomorphic,
//! allocation-free, and exhaustively checked by the compiler when a fourth
//! arrives. Dynamic dispatch is promoted to only when a backend must be added
//! *without editing the enum*, which here means when a filesystem lives in
//! userspace — and at that point the abstraction is the protocol and the trait
//! is irrelevant (D-171).
//!
//! **The resolver cannot ascend, and that is a property of its input.**
//! `libs/path-walk` resolves `..` lexically and refuses underflow, so what
//! arrives here is a list of names that only ever descends. There is no parent
//! pointer anywhere in this file — not a private one, not an unused one — which
//! is stronger than `RESOLVE_BENEATH`: that enforces a predicate on a resolver
//! still *capable* of ascending, and this has no such capability to enforce
//! anything about.
//!
//! The invariant, and it is mutation-tested:
//!
//! > No sequence of path operations from a directory handle can yield a node
//! > that is not a descendant of that handle's node.

use fat32::FatError;
use path_walk::PathError;

/// Components a path may have.
///
/// Sixteen, which is deeper than anything this kernel's one volume holds and
/// bounded because a resolver's work must be bounded by something it chose
/// rather than by the length of an argument ring 3 wrote.
const MAX_DEPTH: usize = 16;

/// A directory, whichever backend it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Directory {
    /// A FAT32 directory, named by the cluster its entries begin at.
    ///
    /// A cluster rather than a node pointer, because FAT has no inode: `012`
    /// §1 says to synthesise identity from the directory entry's location,
    /// which is what GRUB and Linux's `fs/fat` both do.
    Fat {
        /// Where its entries begin.
        cluster: u32,
    },
    /// A directory in the filesystem made of memory.
    Tmp {
        /// Its index in the arena.
        index: usize,
    },
    /// The directory of devices.
    ///
    /// One directory, not a tree: there is nothing to nest, and a devfs with
    /// subdirectories would be a namespace nobody asked for (D-172).
    Dev,
}

/// A file, whichever backend it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Node {
    /// Something with names in it.
    Directory(Directory),
    /// Something with bytes in it.
    File(File),
    /// A device.
    ///
    /// **Not a file**, and the distinction is enforced rather than described:
    /// [`read`] takes a [`File`], so there is no way to pass one of these to
    /// it. Forcing a device to implement `read_at` is the "everything is a
    /// file" conflation, and WASI's own README rejects it on exactly these
    /// grounds (`012` §7, D-172).
    ///
    /// What a program will eventually receive here is slice 4's `Device`
    /// capability. Today the name resolves; the handle it becomes is Phase 7's.
    Device(crate::devfs::Device),
}

/// Why a path did not resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VfsError {
    /// The path itself is not one this kernel can resolve.
    ///
    /// Carries which, because "absolute" and "escapes" are different mistakes
    /// and a caller that cannot tell them apart cannot fix either.
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

/// The directory a program starts from.
///
/// One namespace per program and no way to compose two — `014` carries that as
/// a deliberate hole, and Plan 9's `bind` and Fuchsia's namespace construction
/// are both out of scope. A program gets the directory handles it was given,
/// and this is the one the kernel has to give.
pub(crate) fn root() -> Option<Directory> {
    crate::fs::with_volume(|volume| Ok(volume.bpb().root_cluster()))
        .and_then(Result::ok)
        .map(|cluster| Directory::Fat { cluster })
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

/// Resolve `path` from `at`.
///
/// **The whole walk happens inside one mount**, node to node, holding the
/// backend's state for the walk's life rather than re-deriving a node from a
/// name at each step. That is what closes the gap Linux needed `openat2` for:
/// there is no window between deciding a component is a directory and using
/// it, because nothing is looked up twice.
///
/// # Errors
///
/// See [`VfsError`]. An absolute path and an escaping `..` are both
/// [`VfsError::Path`] — *malformed arguments*, not denials, because there is no
/// root and no parent for them to have been denied (`012` §5).
pub(crate) fn resolve(at: Directory, path: &[u8]) -> Result<Node, VfsError> {
    let mut components = [b"".as_slice(); MAX_DEPTH];
    let depth = path_walk::split(path, &mut components)?;

    // Nothing to walk: the path named the directory it started from, which is
    // what `.` and `a/..` reduce to.
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
                    // The same rule as every other backend's, and the same
                    // reason: a file walked *through* would be answered out of
                    // whatever its bytes look like.
                    return Err(VfsError::NotADirectory);
                }
                return Ok(Node::File(File::Tmp { at, len }));
            }
        }
    }

    Err(VfsError::NotFound)
}

/// Resolve a name among the devices.
///
/// **One component and no more.** A devfs is a directory of names, not a tree,
/// so `storage/anything` is a file walked through — refused for the same reason
/// everywhere else refuses it (D-172).
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
    crate::fs::with_volume(|volume| {
        let mut cluster = from;
        let mut last = None;

        for (index, name) in components.iter().enumerate() {
            let entry = volume.lookup(cluster, name)?;
            let is_last = index + 1 == components.len();

            if !is_last {
                // Every component but the last must be a directory. Without
                // this, `HELLO.TXT/NESTED.TXT` would be answered out of
                // whatever the file's bytes look like as directory entries.
                if !entry.is_directory() {
                    return Err(FatError::WrongKind);
                }
                cluster = entry.cluster;
                continue;
            }

            last = Some(if entry.is_directory() {
                Node::Directory(Directory::Fat {
                    cluster: entry.cluster,
                })
            } else {
                Node::File(File::Fat {
                    cluster: entry.cluster,
                    size: entry.size,
                })
            });
        }

        last.ok_or(FatError::NotFound)
    })
    .ok_or(VfsError::NoVolume)?
    .map_err(VfsError::from)
}

/// Read `into.len()` bytes of `file`, starting `offset` bytes in.
///
/// **Offsets are per operation** (`012` §6, D-152): there is no seek, no tell
/// and no cursor on anything. A shared cursor is a lock on the hot path, it is
/// the only reason two handles to one file would have observable coupling —
/// which in a capability system is a covert channel — and per-operation offsets
/// are what a userspace cursor is *built from*, while the reverse needs a
/// kernel lock.
///
/// # Errors
///
/// See [`VfsError`].
pub(crate) fn read(file: File, offset: u32, into: &mut [u8]) -> Result<usize, VfsError> {
    match file {
        File::Fat { cluster, size } => crate::fs::with_volume(|volume| {
            // Rebuilt rather than kept: a `fat32::Entry` holds a name this
            // does not need, and carrying one would mean a `File` that is a
            // copy of a directory entry rather than a thing in its own right.
            let entry = fat32::Entry::for_contents(cluster, size);
            volume.read_file(&entry, offset, into)
        })
        .ok_or(VfsError::NoVolume)?
        .map_err(VfsError::from),
        File::Tmp { at, len } => Ok(crate::tmpfs::read(at, len, offset as usize, into)),
    }
}

/// Call `found` for every name in `directory`, stopping when it returns false.
///
/// **Enumeration is a separate authority from traversal** (`012` §4): "may
/// resolve a name I know" and "may list the names" are different, and the
/// resolver consults one while this consults the other. Capsicum folds
/// enumeration into read; WASI Preview 1 had both by accident of being
/// syscall-named. Here they are apart on purpose, and slice 9 is where the
/// rights that gate them arrive.
///
/// # Errors
///
/// See [`VfsError`].
pub(crate) fn enumerate(
    directory: Directory,
    mut found: impl FnMut(&[u8], bool) -> bool,
) -> Result<(), VfsError> {
    match directory {
        Directory::Fat { cluster } => crate::fs::with_volume(|volume| {
            volume.walk_directory(cluster, |entry| found(entry.name(), entry.is_directory()))
        })
        .ok_or(VfsError::NoVolume)?
        .map_err(VfsError::from),
        Directory::Tmp { index } => {
            crate::tmpfs::walk(index, found);
            Ok(())
        }
        Directory::Dev => {
            crate::devfs::walk(found);
            Ok(())
        }
    }
}
