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
//! **Fixed-size and read-only before the Wizzy gate.** The arena is a `static`;
//! admitting growth here would invent the post-gate write protocol before its
//! caller. What is here is populated once, at boot.

use wost::sync::{Once, SpinLock};

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

impl Arena {
    const EMPTY: Self = Self {
        entries: [Entry::EMPTY; MAX_ENTRIES],
        used: 0,
        bytes: [0; MAX_BYTES],
        written: 0,
    };
}

/// The one instance.
static ARENA: SpinLock<Arena> = SpinLock::new(Arena::EMPTY);
static INIT: Once = Once::new();

/// Build the tree this kernel starts with.
pub(crate) fn init() {
    assert!(INIT.claim(), "tmpfs initialized twice");
    let mut arena = Arena::EMPTY;

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
    add(
        &mut arena,
        notes,
        b"ONE.TXT",
        Some(b"a file in a directory\n"),
    );
    // Duplicate names in different parents expose parent-blind lookup.
    add(&mut arena, notes, b"MOTD", Some(b"a different motd\n"));

    let mut stored = ARENA.lock_leaf();
    *stored = arena;
}

fn add(arena: &mut Arena, parent: usize, name: &[u8], content: Option<&[u8]>) -> usize {
    assert!(
        arena.used < MAX_ENTRIES,
        "tmpfs entry capacity is structural"
    );
    assert!(
        !name.is_empty() && name.len() <= MAX_NAME,
        "invalid tmpfs boot name"
    );
    assert!(
        parent < arena.used && matches!(arena.entries[parent].kind, Kind::Directory),
        "tmpfs boot parent is not a directory"
    );
    assert!(
        arena.entries[..arena.used]
            .iter()
            .all(|entry| { entry.parent != parent || &entry.name[..entry.name_len] != name }),
        "duplicate tmpfs boot name"
    );
    let next_written = content.map(|bytes| {
        let Some(end) = arena.written.checked_add(bytes.len()) else {
            panic!("tmpfs byte capacity overflowed");
        };
        assert!(end <= MAX_BYTES, "tmpfs byte capacity is structural");
        end
    });
    let kind = match content {
        None => Kind::Directory,
        Some(bytes) => {
            let at = arena.written;
            let Some(end) = next_written else {
                panic!("file bytes were not preflighted");
            };
            arena.bytes[at..end].copy_from_slice(bytes);
            arena.written = end;
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
    index
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
        // The parent is the whole containment check; `index != ROOT` stops
        // the root, its own parent, from being found inside itself.
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

/// One owned directory-entry summary.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Summary {
    name: [u8; MAX_NAME],
    name_len: usize,
    directory: bool,
}

impl Summary {
    const EMPTY: Self = Self {
        name: [0; MAX_NAME],
        name_len: 0,
        directory: false,
    };

    pub(crate) fn name(&self) -> &[u8] {
        &self.name[..self.name_len]
    }

    pub(crate) const fn is_directory(&self) -> bool {
        self.directory
    }
}

/// Owned children of one directory.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Listing {
    entries: [Summary; MAX_ENTRIES - 1],
    len: usize,
}

impl Listing {
    pub(crate) fn entries(&self) -> &[Summary] {
        &self.entries[..self.len]
    }
}

/// Copy every child out while the arena is locked.
pub(crate) fn list(directory: usize) -> Option<Listing> {
    let arena = ARENA.lock_leaf();
    if directory >= arena.used || !matches!(arena.entries[directory].kind, Kind::Directory) {
        return None;
    }
    let mut listing = Listing {
        entries: [Summary::EMPTY; MAX_ENTRIES - 1],
        len: 0,
    };
    for index in 0..arena.used {
        let entry = &arena.entries[index];
        if index == ROOT || entry.parent != directory {
            continue;
        }
        listing.entries[listing.len] = Summary {
            name: entry.name,
            name_len: entry.name_len,
            directory: matches!(entry.kind, Kind::Directory),
        };
        listing.len += 1;
    }
    Some(listing)
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
    let wanted = into.len().min(len - offset);
    let Some(from) = at.checked_add(offset) else {
        return 0;
    };
    let Some(end) = from.checked_add(wanted) else {
        return 0;
    };
    if end > MAX_BYTES {
        return 0;
    }
    let arena = ARENA.lock_leaf();
    if end > arena.written {
        return 0;
    }
    into[..wanted].copy_from_slice(&arena.bytes[from..end]);
    wanted
}

/// Check that fabricated file metadata cannot expose unwritten arena bytes.
#[cfg(feature = "selftest")]
pub(crate) fn unwritten_bytes_are_refused_for_test() -> bool {
    let mut byte = [0xA5];
    read(MAX_BYTES - 1, 1, 0, &mut byte) == 0 && byte == [0xA5]
}
