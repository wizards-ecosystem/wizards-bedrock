use wost::sync::SpinLock;

use crate::devfs;
use crate::fs;
use crate::nvme;
use crate::tmpfs;
use crate::vfs;

pub(super) fn a_second_backend_answers_the_same_three_operations() -> bool {
    let root = vfs::tmp_root();

    // Resolve and read a file at the top.
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

    // A read from the middle starts where it was told, and stops at the end
    // rather than at the arena's.
    let mut window = [0u8; 8];
    let Ok(partial) = vfs::read(motd, 4, &mut window) else {
        return false;
    };
    if partial != window.len() || window[..] != read[4..12] {
        return false;
    }
    if vfs::read(motd, got as u32, &mut window) != Ok(0) {
        return false;
    }

    // Resolve through a directory, which is the walk rather than a lookup.
    let Ok(vfs::Node::File(nested)) = vfs::resolve(root, b"NOTES/ONE.TXT") else {
        return false;
    };
    let Ok(inside) = vfs::read(nested, 0, &mut read) else {
        return false;
    };
    if &read[..inside] != b"a file in a directory\n" {
        return false;
    }

    // A name in two directories resolves to the right one; a lookup ignoring
    // the parent is only wrong when names repeat, so here they do (D-172).
    let Ok(vfs::Node::File(shadowed)) = vfs::resolve(root, b"NOTES/MOTD") else {
        return false;
    };
    let Ok(shadow) = vfs::read(shadowed, 0, &mut read) else {
        return false;
    };
    if &read[..shadow] != b"a different motd\n" {
        return false;
    }

    // And enumeration agrees with what resolves.
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

    // A file walked through is refused here too, which is the rule being the
    // VFS's rather than FAT's.
    let through = vfs::resolve(root, b"MOTD/ONE.TXT");
    let hostile = vfs::read(
        vfs::File::Tmp {
            at: usize::MAX,
            len: usize::MAX,
        },
        1,
        &mut window,
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

pub(super) fn a_device_is_not_a_file() -> bool {
    let root = vfs::dev_root();

    let Ok(vfs::Node::Device(storage)) = vfs::resolve(root, b"storage") else {
        return false;
    };
    if storage.name() != b"storage" {
        return false;
    }
    // Not a file and not a directory: the *node* is a third thing, which is
    // what makes the distinction structural rather than a flag.
    if matches!(vfs::resolve(root, b"console"), Ok(vfs::Node::File(_))) {
        return false;
    }
    let Ok(vfs::Node::Device(console)) = vfs::resolve(root, b"console") else {
        return false;
    };
    if console == storage {
        return false;
    }

    // Every device is listed, and none of them claims to be a directory.
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

    // A device has nothing inside it, so a path through one is refused for the
    // same reason a path through a file is.
    let through = vfs::resolve(root, b"storage/anything");
    let missing = vfs::resolve(root, b"nothing");

    listed == devfs::DEVICES.len()
        && !any_directory
        && matches!(through, Err(vfs::VfsError::NotADirectory))
        && matches!(missing, Err(vfs::VfsError::NotFound))
}

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

    // Names that exist on one backend and not another. Each of these resolves
    // from its own root and from nowhere else.
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

    // And each name does resolve where it belongs, so the refusals above are
    // about the backend rather than about the name.
    vfs::resolve(vfs::tmp_root(), b"MOTD").is_ok()
        && vfs::resolve(vfs::dev_root(), b"storage").is_ok()
}

pub(super) fn a_path_resolves_from_a_directory_handle() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    // A file at the top.
    let Ok(vfs::Node::File(small)) = vfs::resolve(root, SMALL_NAME) else {
        return false;
    };
    // The variant matters: a FAT root resolves to a FAT file, and the match
    // is the enum dispatch being exhaustive (D-172).
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

    // A directory, then a file inside it, through one path.
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

    // And `.` and a cancelled `..` both name the directory they started from.
    let here = matches!(vfs::resolve(root, b"."), Ok(vfs::Node::Directory(_)));
    let round_trip = matches!(vfs::resolve(root, b"DATA/.."), Ok(vfs::Node::Directory(_)));

    got == NESTED_CONTENT.len() && &inside[..got] == NESTED_CONTENT && here && round_trip
}

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

    // And an empty path names nothing rather than the directory itself, which
    // is what a resolver that treated it as `.` would answer.
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

    // Resolve after enumeration because both paths take the controller leaf;
    // resolving in the callback would re-enter it (D-171).
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

pub(super) fn the_kernel_reads_the_file_the_host_wrote() -> bool {
    matches!(
        fs::mount().map(|result| result.and_then(|mut volume| {
            let root = volume.root_cluster();
            let entry = volume.resolve(root, &[SMALL_NAME])?;
            if entry.size as usize != SMALL_CONTENT.len() || entry.is_directory() {
                return Ok(false);
            }

            // A deleted slot separates live entries, so traversal must continue.
            let mut live = 0usize;
            let mut cursor = fat32::DirectoryCursor::new(root);
            while !cursor.is_done() {
                live += volume.directory_chunk(&mut cursor)?.entries().len();
            }
            if live != 3 {
                return Ok(false);
            }

            // Extra buffer exposes a read incorrectly bounded by cluster size
            // instead of file size (D-170).
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

            // And nothing past the end, rather than the next file's bytes.
            let mut past = [0u8; 16];
            Ok(volume.read_file(entry.cluster, entry.size, entry.size, &mut past)? == 0)
        })),
        Some(Ok(true))
    )
}

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

            // A file is not a directory, whatever its bytes look like.
            let through_a_file = volume.resolve(root, &[SMALL_NAME, NESTED_NAME]);
            let missing = volume.resolve(root, &[b"NOPE.TXT"]);
            Ok(through_a_file == Err(fat32::FatError::WrongKind)
                && missing == Err(fat32::FatError::NotFound))
        })),
        Some(Ok(true))
    )
}

/// What `xtask` put on the namespace.
///
/// Transcribed from `xtask/src/image/corpus.rs`, which writes it — two copies
/// of one table, so a host test compares them (D-158, D-170).
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
