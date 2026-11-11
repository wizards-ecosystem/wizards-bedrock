use wost::sync::SpinLock;

use crate::devfs;
use crate::fs;
use crate::nvme;
use crate::tmpfs;
use crate::vfs;

pub(super) fn a_second_backend_answers_the_same_three_operations() -> bool {
    let root = vfs::tmp_root();
    let mut window = [0u8; 8];
    tmp_files_read_back(root, &mut window) && tmp_listing_agrees_and_walks_stop(root, &mut window)
}

// A read from the middle starts at its offset and stops at the file's end, before the arena's.
// A lookup ignoring the parent is only wrong when names repeat, so `NOTES/MOTD` repeats one.
fn tmp_files_read_back(root: vfs::Directory, window: &mut [u8; 8]) -> bool {
    let Ok(vfs::Node::File(motd)) = vfs::resolve(root, b"MOTD") else {
        return false;
    };
    let mut read = [0u8; 64];
    let Ok(got) = vfs::read(motd, 0, &mut read) else {
        return false;
    };
    if got == 0 || &read[..got] != b"the wizard's bedrock, from memory\n" {
        return false;
    }

    let Ok(partial) = vfs::read(motd, 4, window) else {
        return false;
    };
    if partial != window.len() || window[..] != read[4..12] {
        return false;
    }
    if vfs::read(motd, got as u32, window) != Ok(0) {
        return false;
    }

    let Ok(vfs::Node::File(nested)) = vfs::resolve(root, b"NOTES/ONE.TXT") else {
        return false;
    };
    let Ok(inside) = vfs::read(nested, 0, &mut read) else {
        return false;
    };
    if &read[..inside] != b"a file in a directory\n" {
        return false;
    }

    let Ok(vfs::Node::File(shadowed)) = vfs::resolve(root, b"NOTES/MOTD") else {
        return false;
    };
    let Ok(shadow) = vfs::read(shadowed, 0, &mut read) else {
        return false;
    };
    &read[..shadow] == b"a different motd\n"
}

// Walking through a file is refused here too, because that rule belongs to the VFS.
fn tmp_listing_agrees_and_walks_stop(root: vfs::Directory, window: &mut [u8; 8]) -> bool {
    let mut names = 0usize;
    let mut directories = 0usize;
    if vfs::enumerate(root, |_, is_directory| {
        names += 1;
        if is_directory {
            directories += 1;
        }
        true
    })
    .is_err()
    {
        return false;
    }

    let through = vfs::resolve(root, b"MOTD/ONE.TXT");
    let hostile = vfs::read(
        vfs::File::Tmp {
            at: usize::MAX,
            len: usize::MAX,
        },
        1,
        window,
    );
    names == 2
        && directories == 1
        && matches!(through, Err(vfs::VfsError::NotADirectory))
        && hostile == Ok(0)
        && tmpfs::unwritten_bytes_are_refused_for_test()
}

static ENUMERATION_SENTINEL: SpinLock<()> = SpinLock::new(());

pub(super) fn enumeration_callbacks_run_without_backend_ownership() -> bool {
    let tmp = vfs::tmp_root();
    let mut tmp_reentered = false;
    let tmp_walked = vfs::enumerate(tmp, |name, _| {
        {
            let _sentinel = ENUMERATION_SENTINEL.lock_leaf();
        }
        if wost::sync::taken_under_a_leaf().is_some() {
            return false;
        }
        tmp_reentered = vfs::resolve(tmp, name).is_ok();
        false
    });

    let Some(fat) = vfs::root() else {
        return false;
    };
    let mut fat_reentered = false;
    let fat_walked = vfs::enumerate(fat, |name, _| {
        if !nvme::controller_is_unlocked_for_test() {
            return false;
        }
        fat_reentered = vfs::resolve(fat, name).is_ok();
        false
    });

    tmp_walked.is_ok() && tmp_reentered && fat_walked.is_ok() && fat_reentered
}

// A device is a third kind of node, which makes the distinction structural.
// A path through a device is refused as one through a file is, since it has nothing inside.
pub(super) fn a_device_is_not_a_file() -> bool {
    let root = vfs::dev_root();

    let Ok(vfs::Node::Device(storage)) = vfs::resolve(root, b"storage") else {
        return false;
    };
    if storage.name() != b"storage" {
        return false;
    }
    if matches!(vfs::resolve(root, b"console"), Ok(vfs::Node::File(_))) {
        return false;
    }
    let Ok(vfs::Node::Device(console)) = vfs::resolve(root, b"console") else {
        return false;
    };
    if console == storage {
        return false;
    }

    let mut listed = 0usize;
    let mut any_directory = false;
    if vfs::enumerate(root, |_, is_directory| {
        listed += 1;
        any_directory |= is_directory;
        true
    })
    .is_err()
    {
        return false;
    }

    let through = vfs::resolve(root, b"storage/anything");
    let missing = vfs::resolve(root, b"nothing");

    listed == devfs::DEVICES.len()
        && !any_directory
        && matches!(through, Err(vfs::VfsError::NotADirectory))
        && matches!(missing, Err(vfs::VfsError::NotFound))
}

// Each crossing name resolves on its own backend, so its refusal elsewhere is about the backend.
pub(super) fn no_backend_lets_a_path_escape_or_cross() -> bool {
    let escaping: &[&[u8]] = &[b"..", b"../MOTD", b"NOTES/../..", b"./.."];
    let absolute: &[&[u8]] = &[b"/", b"/MOTD"];

    for root in [vfs::tmp_root(), vfs::dev_root()] {
        for path in escaping {
            if !matches!(
                vfs::resolve(root, path),
                Err(vfs::VfsError::Path(path_walk::PathError::Escapes))
            ) {
                return false;
            }
        }
        for path in absolute {
            if !matches!(
                vfs::resolve(root, path),
                Err(vfs::VfsError::Path(path_walk::PathError::Absolute))
            ) {
                return false;
            }
        }
    }

    let crossings: &[(vfs::Directory, &[u8])] = &[
        (vfs::tmp_root(), SMALL_NAME),
        (vfs::tmp_root(), b"storage"),
        (vfs::dev_root(), b"MOTD"),
        (vfs::dev_root(), SMALL_NAME),
    ];
    for (root, name) in crossings {
        if !matches!(vfs::resolve(*root, name), Err(vfs::VfsError::NotFound)) {
            return false;
        }
    }

    vfs::resolve(vfs::tmp_root(), b"MOTD").is_ok()
        && vfs::resolve(vfs::dev_root(), b"storage").is_ok()
}

// A FAT root must resolve to a FAT file, through the exhaustive enum dispatch.
pub(super) fn a_path_resolves_from_a_directory_handle() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    let Ok(vfs::Node::File(small)) = vfs::resolve(root, SMALL_NAME) else {
        return false;
    };
    let vfs::File::Fat { size, .. } = small else {
        return false;
    };
    if size as usize != SMALL_CONTENT.len() {
        return false;
    }
    let mut read = [0u8; fat32::SECTOR_BYTES];
    let Ok(got) = vfs::read(small, 0, &mut read) else {
        return false;
    };
    if got != SMALL_CONTENT.len() || &read[..got] != SMALL_CONTENT {
        return false;
    }

    let Ok(vfs::Node::Directory(_)) = vfs::resolve(root, DIRECTORY_NAME) else {
        return false;
    };
    let mut deeper = [0u8; 32];
    deeper[..DIRECTORY_NAME.len()].copy_from_slice(DIRECTORY_NAME);
    deeper[DIRECTORY_NAME.len()] = b'/';
    let tail = DIRECTORY_NAME.len() + 1;
    deeper[tail..tail + NESTED_NAME.len()].copy_from_slice(NESTED_NAME);
    let path = &deeper[..tail + NESTED_NAME.len()];

    let Ok(vfs::Node::File(nested)) = vfs::resolve(root, path) else {
        return false;
    };
    let mut inside = [0u8; 64];
    let Ok(got) = vfs::read(nested, 0, &mut inside) else {
        return false;
    };

    let here = matches!(vfs::resolve(root, b"."), Ok(vfs::Node::Directory(_)));
    let round_trip = matches!(vfs::resolve(root, b"DATA/.."), Ok(vfs::Node::Directory(_)));

    got == NESTED_CONTENT.len() && &inside[..got] == NESTED_CONTENT && here && round_trip
}

// An empty path names nothing; a resolver treating it as `.` would answer the directory.
pub(super) fn no_path_can_name_anything_above_the_handle() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    let absolute: &[&[u8]] = &[b"/", b"/DATA", b"//DATA", b"/../DATA", b"/DATA/NESTED.TXT"];
    for path in absolute {
        if !matches!(
            vfs::resolve(root, path),
            Err(vfs::VfsError::Path(path_walk::PathError::Absolute))
        ) {
            return false;
        }
    }

    let escaping: &[&[u8]] = &[
        b"..",
        b"../DATA",
        b"DATA/../..",
        b"DATA/../../DATA",
        b"./..",
        b"DATA/./../..",
        b"DATA/../DATA/../..",
    ];
    for path in escaping {
        if !matches!(
            vfs::resolve(root, path),
            Err(vfs::VfsError::Path(path_walk::PathError::Escapes))
        ) {
            return false;
        }
    }

    matches!(
        vfs::resolve(root, b""),
        Err(vfs::VfsError::Path(path_walk::PathError::Empty))
    )
}

pub(super) fn a_file_cannot_be_walked_through() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    let mut path = [0u8; 32];
    path[..SMALL_NAME.len()].copy_from_slice(SMALL_NAME);
    path[SMALL_NAME.len()] = b'/';
    let tail = SMALL_NAME.len() + 1;
    path[tail..tail + NESTED_NAME.len()].copy_from_slice(NESTED_NAME);

    let through = vfs::resolve(root, &path[..tail + NESTED_NAME.len()]);
    let missing = vfs::resolve(root, b"NOPE.TXT");

    matches!(through, Err(vfs::VfsError::NotADirectory))
        && matches!(missing, Err(vfs::VfsError::NotFound))
}

// Names resolve only after enumeration: both take the controller leaf, so the callback would
// re-enter it.
pub(super) fn enumeration_lists_what_traversal_would_find() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    let mut names = 0usize;
    let mut directories = 0usize;
    let mut saw_small = false;
    let mut saw_large = false;
    if vfs::enumerate(root, |name, is_directory| {
        names += 1;
        if is_directory {
            directories += 1;
        }
        saw_small |= name == SMALL_NAME;
        saw_large |= name == LARGE_NAME;
        true
    })
    .is_err()
    {
        return false;
    }

    let mut listed = [[0u8; 12]; 4];
    let mut lengths = [0usize; 4];
    let mut count = 0usize;
    let listed_all = vfs::enumerate(root, |name, _| {
        if count < listed.len() && name.len() <= listed[count].len() {
            listed[count][..name.len()].copy_from_slice(name);
            lengths[count] = name.len();
            count += 1;
        }
        true
    })
    .is_ok();

    let mut all_resolve = listed_all && count == 3;
    for index in 0..count {
        all_resolve &= vfs::resolve(root, &listed[index][..lengths[index]]).is_ok();
    }

    names == 3 && directories == 1 && saw_small && saw_large && all_resolve
}

pub(super) fn the_volume_mounts_and_says_what_a_fat32_volume_says() -> bool {
    matches!(fs::mount(), Some(Ok(volume)) if volume.layout_is_fat32())
}

// A deleted slot separates live entries, so traversal must continue past it.
// The sector-sized buffer exposes a read wrongly bounded by the cluster size.
pub(super) fn the_kernel_reads_the_file_the_host_wrote() -> bool {
    matches!(
        fs::mount().map(|result| result.and_then(|mut volume| {
            let root = volume.root_cluster();
            let entry = volume.resolve(root, &[SMALL_NAME])?;
            if entry.size as usize != SMALL_CONTENT.len() || entry.is_directory() {
                return Ok(false);
            }

            let mut live = 0usize;
            let mut cursor = fat32::DirectoryCursor::new(root);
            while !cursor.is_done() {
                live += volume.directory_chunk(&mut cursor)?.entries().len();
            }
            if live != 3 {
                return Ok(false);
            }

            let mut read = [0u8; fat32::SECTOR_BYTES];
            let got = volume.read_file(entry.cluster, entry.size, 0, &mut read)?;
            Ok(got == SMALL_CONTENT.len() && &read[..got] == SMALL_CONTENT)
        })),
        Some(Ok(true))
    )
}

pub(super) fn a_file_longer_than_a_cluster_is_read_across_its_chain() -> bool {
    matches!(
        fs::mount().map(|result| result.and_then(|mut volume| {
            let root = volume.root_cluster();
            let entry = volume.resolve(root, &[LARGE_NAME])?;
            if entry.size as usize != LARGE_BYTES {
                return Ok(false);
            }
            let cluster = volume.shape().1 as usize;

            for offset in [
                0usize,
                1,
                fat32::SECTOR_BYTES - 1,
                cluster - 1,
                cluster,
                cluster + 1,
                cluster * 2 + 13,
                LARGE_BYTES - 64,
            ] {
                let mut window = [0u8; 64];
                let got =
                    volume.read_file(entry.cluster, entry.size, offset as u32, &mut window)?;
                if got != window.len() {
                    return Ok(false);
                }
                for (index, byte) in window.iter().enumerate() {
                    if *byte != large_byte(offset + index) {
                        return Ok(false);
                    }
                }
            }

            let mut past = [0u8; 16];
            Ok(volume.read_file(entry.cluster, entry.size, entry.size, &mut past)? == 0)
        })),
        Some(Ok(true))
    )
}

// A file is never walked as a directory, whatever its bytes look like.
pub(super) fn a_path_resolves_through_a_directory() -> bool {
    matches!(
        fs::mount().map(|result| result.and_then(|mut volume| {
            let root = volume.root_cluster();
            let nested = volume.resolve(root, &[DIRECTORY_NAME, NESTED_NAME])?;
            if nested.size as usize != NESTED_CONTENT.len() {
                return Ok(false);
            }
            let mut read = [0u8; 128];
            let got = volume.read_file(nested.cluster, nested.size, 0, &mut read)?;
            if got != NESTED_CONTENT.len() || &read[..got] != NESTED_CONTENT {
                return Ok(false);
            }

            let through_a_file = volume.resolve(root, &[SMALL_NAME, NESTED_NAME]);
            let missing = volume.resolve(root, &[b"NOPE.TXT"]);
            Ok(through_a_file == Err(fat32::FatError::WrongKind)
                && missing == Err(fat32::FatError::NotFound))
        })),
        Some(Ok(true))
    )
}

/// What `xtask` put on the namespace, transcribed from `xtask/src/image/corpus.rs`, which
/// writes it; a host test compares the two copies.
pub(super) const SMALL_NAME: &[u8] = b"HELLO.TXT";
/// See [`SMALL_NAME`].
pub(super) const SMALL_CONTENT: &[u8] = b"the wizard's bedrock reads a disk\n";
/// See [`SMALL_NAME`].
const LARGE_NAME: &[u8] = b"BIG.BIN";
/// See [`SMALL_NAME`].
const LARGE_BYTES: usize = 40 * 1024;
/// See [`SMALL_NAME`].
pub(super) const DIRECTORY_NAME: &[u8] = b"DATA";
/// See [`SMALL_NAME`].
const NESTED_NAME: &[u8] = b"NESTED.TXT";
/// See [`SMALL_NAME`].
const NESTED_CONTENT: &[u8] = b"one directory down\n";

/// Byte `index` of [`LARGE_NAME`], as `xtask` computed it.
const fn large_byte(index: usize) -> u8 {
    ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
}
