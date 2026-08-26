//! A filesystem made of memory.
//!
//! The second backend, and `014` says why there has to be one: *the VFS needs
//! at least two backends before its abstraction is anything but a guess.* A
//! directory trait with one implementation is a description of that
//! implementation.
//!
//! **Deliberately unlike FAT32 in the ways that matter.** It has no clusters,
//! no chains, no on-disk anything, and its directories are a flat table with
//! parent indices rather than a linked structure on a device. If the VFS's
//! three operations fit both, they are about directories rather than about
//! FAT.
//!
//! **Fixed-size, and read-only for now.** The arena is a `static`, because the
//! write path is Phase 6's largest deliberate hole (`014`) and a tmpfs that
//! could grow would be the write path arriving through the back door. What is
//! here is populated once, at boot.

use wost::sync::SpinLock;

/// Entries the arena holds.
const MAX_ENTRIES: usize = 8;

/// Bytes of file content the arena holds.
const MAX_BYTES: usize = 512;

/// The longest name an entry may have.
const MAX_NAME: usize = 16;

/// The index of the root directory, which always exists.
pub(crate) const ROOT: usize = 0;

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A directory: other entries name it as their parent.
    Directory,
    /// A file, as a range of the byte arena.
    File { at: usize, len: usize },
}

/// One entry.
#[derive(Debug, Clone, Copy)]
struct Entry {
    name: [u8; MAX_NAME],
    name_len: usize,
    parent: usize,
    kind: Kind,
}

impl Entry {
    /// The unused slot.
    const EMPTY: Self = Self {
        name: [0; MAX_NAME],
        name_len: 0,
        parent: usize::MAX,
        kind: Kind::Directory,
    };
}

/// The whole filesystem.
#[derive(Debug)]
struct Arena {
    entries: [Entry; MAX_ENTRIES],
    used: usize,
    bytes: [u8; MAX_BYTES],
    written: usize,
}

/// The one instance.
static ARENA: SpinLock<Arena> = SpinLock::new(Arena {
    entries: [Entry::EMPTY; MAX_ENTRIES],
    used: 0,
    bytes: [0; MAX_BYTES],
    written: 0,
});

/// Build the tree this kernel starts with.
///
/// Idempotent: a second call finds the root already there and does nothing,
/// because the question it is really asking is "is there a tmpfs", and there
/// is.
pub(crate) fn init() {
    let mut arena = ARENA.lock_leaf();
    if arena.used > 0 {
        return;
    }

    // The root, which is its own parent — the only entry that is, and the
    // reason nothing needs a separate "is the root" flag.
    arena.entries[ROOT] = Entry {
        name: [0; MAX_NAME],
        name_len: 0,
        parent: ROOT,
        kind: Kind::Directory,
    };
    arena.used = 1;

    add(
        &mut arena,
        ROOT,
        b"MOTD",
        Some(b"the wizard's os, from memory\n"),
    );
    let notes = add(&mut arena, ROOT, b"NOTES", None);
    if let Some(notes) = notes {
        add(
            &mut arena,
            notes,
            b"ONE.TXT",
            Some(b"a file in a directory\n"),
        );
        // **The same name as the root's, on purpose.** A lookup that ignored
        // the parent would find whichever came first and be right about every
        // path in a tree whose names are unique — so this tree's are not
        // (D-155, D-172).
        add(&mut arena, notes, b"MOTD", Some(b"a different motd\n"));
    }
}

/// Add an entry, and say where it went.
fn add(arena: &mut Arena, parent: usize, name: &[u8], content: Option<&[u8]>) -> Option<usize> {
    if arena.used >= MAX_ENTRIES || name.len() > MAX_NAME {
        return None;
    }
    let kind = match content {
        None => Kind::Directory,
        Some(bytes) => {
            if arena.written + bytes.len() > MAX_BYTES {
                return None;
            }
            let at = arena.written;
            arena.bytes[at..at + bytes.len()].copy_from_slice(bytes);
            arena.written += bytes.len();
            Kind::File {
                at,
                len: bytes.len(),
            }
        }
    };

    let index = arena.used;
    let mut stored = [0u8; MAX_NAME];
    stored[..name.len()].copy_from_slice(name);
    arena.entries[index] = Entry {
        name: stored,
        name_len: name.len(),
        parent,
        kind,
    };
    arena.used += 1;
    Some(index)
}

/// What a name in a directory resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Found {
    /// A directory, by index.
    Directory(usize),
    /// A file: where its bytes are and how many there are.
    File { at: usize, len: usize },
}

/// Find `name` in the directory at `directory`.
pub(crate) fn lookup(directory: usize, name: &[u8]) -> Option<Found> {
    let arena = ARENA.lock_leaf();
    for index in 0..arena.used {
        let entry = &arena.entries[index];
        // **The parent is the whole of the containment check**, and `index !=
        // ROOT` is what stops the root — which is its own parent — from being
        // found inside itself under an empty name.
        if index == ROOT || entry.parent != directory {
            continue;
        }
        if &entry.name[..entry.name_len] != name {
            continue;
        }
        return Some(match entry.kind {
            Kind::Directory => Found::Directory(index),
            Kind::File { at, len } => Found::File { at, len },
        });
    }
    None
}

/// Call `found` for every entry of `directory`.
pub(crate) fn walk(directory: usize, mut found: impl FnMut(&[u8], bool) -> bool) {
    let arena = ARENA.lock_leaf();
    for index in 0..arena.used {
        let entry = &arena.entries[index];
        if index == ROOT || entry.parent != directory {
            continue;
        }
        let is_directory = matches!(entry.kind, Kind::Directory);
        if !found(&entry.name[..entry.name_len], is_directory) {
            return;
        }
    }
}

/// Copy bytes of a file into `into`, starting `offset` in.
///
/// Returns how many were copied, which is fewer than asked for at the end of
/// the file and zero past it — the same shape as every other backend's, and
/// with the same reason: **offsets are per operation** and nothing here holds
/// a cursor (D-152, D-171).
pub(crate) fn read(at: usize, len: usize, offset: usize, into: &mut [u8]) -> usize {
    if offset >= len {
        return 0;
    }
    let arena = ARENA.lock_leaf();
    let wanted = into.len().min(len - offset);
    let from = at + offset;
    if from + wanted > arena.bytes.len() {
        return 0;
    }
    into[..wanted].copy_from_slice(&arena.bytes[from..from + wanted]);
    wanted
}
