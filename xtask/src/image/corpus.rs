//! Deterministic FAT32 corpus shared by machine and host checks.
//!
//! The exact bytes embedded in the NVMe namespace are read independently by
//! `fatfs` and `libs/fat32`; the pinned foreign corpus separately checks the
//! format assumptions (D-170, D-207).

use std::io::{Read, Seek, Write};

use anyhow::{Context, Result};
use fatfs::{FatType, FileSystem, FsOptions};

use super::esp;

/// Bytes of the volume.
///
/// FAT32 needs at least 65525 clusters to be FAT32 at all, so a volume much
/// smaller than this is silently formatted as FAT16 — which this reader
/// refuses, correctly, and which would look like a bug in the reader.
pub const VOLUME_BYTES: usize = 64 * 1024 * 1024;

/// A file small enough to sit in one cluster.
pub const SMALL_NAME: &str = "HELLO.TXT";

/// A file long enough to need a chain of them.
///
/// Forty kilobytes: more than one cluster at every cluster size `fatfs` will
/// choose here, so reading it walks the table rather than staying in the first
/// cluster. A file that fitted would leave the chain walk unexercised.
pub const LARGE_NAME: &str = "BIG.BIN";

/// Bytes of [`LARGE_NAME`].
pub const LARGE_BYTES: usize = 40 * 1024;

/// A file that is created and then deleted.
///
/// It leaves a slot beginning `0xE5` **before** live entries, which is the
/// only way to reach the difference between "deleted" and "the directory ends
/// here" on a real volume. A reader that confused the two would lose every
/// file after it, and against a corpus with no deletions nothing would say so
/// (D-170).
pub const DELETED_NAME: &str = "GONE.TXT";

/// A directory, so the walk has somewhere to descend.
pub const DIRECTORY_NAME: &str = "DATA";

/// A file inside it.
pub const NESTED_NAME: &str = "NESTED.TXT";

/// What [`SMALL_NAME`] holds.
pub const SMALL_CONTENT: &[u8] = b"the wizard's bedrock reads a disk\n";

/// What [`NESTED_NAME`] holds.
pub const NESTED_CONTENT: &[u8] = b"one directory down\n";

/// Byte `index` of [`LARGE_NAME`].
///
/// **Position-dependent on purpose.** A file of one repeated byte cannot tell a
/// read at the right offset from a read at the wrong one, and a reader that
/// walked the chain incorrectly would return plausible bytes from the wrong
/// cluster and pass. This is D-155's rule applied to a corpus (D-170).
pub const fn large_byte(index: usize) -> u8 {
    // A cheap mixing of the index so neighbouring bytes differ and so do
    // bytes a cluster apart.
    ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
}

/// Build the volume.
///
/// # Errors
///
/// If `fatfs` refuses to format or to write, which on an in-memory volume of
/// this size means a version change rather than an environment problem.
#[cfg(test)]
fn build() -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; VOLUME_BYTES];
    populate(std::io::Cursor::new(&mut bytes[..]))?;

    Ok(bytes)
}

pub(crate) fn populate(mut volume: impl Read + Write + Seek) -> Result<()> {
    esp::format(&mut volume, FatType::Fat32).context("formatting the corpus volume")?;
    let filesystem =
        FileSystem::new(volume, FsOptions::new()).context("opening the corpus volume")?;
    {
        let root = filesystem.root_dir();

        esp::write_file(&root, SMALL_NAME, SMALL_CONTENT)?;
        // Removing this only after later live entries exist makes premature
        // end-of-directory handling observable.
        esp::write_file(&root, DELETED_NAME, b"not for long\n")?;

        let large: Vec<u8> = (0..LARGE_BYTES).map(large_byte).collect();
        esp::write_file(&root, LARGE_NAME, &large)?;

        let directory = root
            .create_dir(DIRECTORY_NAME)
            .with_context(|| format!("creating /{DIRECTORY_NAME}"))?;
        esp::write_file(&directory, NESTED_NAME, NESTED_CONTENT)?;

        root.remove(DELETED_NAME)
            .with_context(|| format!("removing /{DELETED_NAME}"))?;
    }
    filesystem.unmount().context("unmounting the corpus")
}

#[cfg(test)]
mod tests;
