use wost::sync::{Once, SpinLock};

const MAX_ENTRIES: usize = 8;

const MAX_BYTES: usize = 512;

const MAX_NAME: usize = 16;

pub(crate) const ROOT: usize = 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Directory,
    File { at: usize, len: usize },
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    name: [u8; MAX_NAME],
    name_len: usize,
    parent: usize,
    kind: Kind,
}

impl Entry {
    const EMPTY: Self = Self {
        name: [0; MAX_NAME],
        name_len: 0,
        parent: usize::MAX,
        kind: Kind::Directory,
    };
}

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

static ARENA: SpinLock<Arena> = SpinLock::new(Arena::EMPTY);
static INIT: Once = Once::new();

pub(crate) fn init() {
    assert!(INIT.claim(), "tmpfs initialized twice");
    let mut arena = Arena::EMPTY;

    // The root is its own parent, the only entry that is, so no "is the root" flag is needed.
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
        Some(b"the wizard's bedrock, from memory\n"),
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
    let kind = match content {
        None => Kind::Directory,
        Some(bytes) => {
            let at = arena.written;
            let Some(end) = at.checked_add(bytes.len()) else {
                panic!("tmpfs byte capacity overflowed");
            };
            assert!(end <= MAX_BYTES, "tmpfs byte capacity is structural");
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Found {
    Directory(usize),
    File { at: usize, len: usize },
}

pub(crate) fn lookup(directory: usize, name: &[u8]) -> Option<Found> {
    let arena = ARENA.lock_leaf();
    for index in 0..arena.used {
        let entry = &arena.entries[index];
        // The parent is the whole containment check; skipping `ROOT` keeps it out of itself.
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

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct Listing {
    entries: [Summary; MAX_ENTRIES - 1],
    len: usize,
}

impl Listing {
    pub(crate) fn entries(&self) -> &[Summary] {
        &self.entries[..self.len]
    }
}

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
