//! Deterministic FAT32 corpus that `fatfs` and `libs/fat32` read independently.

use std::io::{Read, Seek, Write};

use anyhow::{Context, Result};
use fatfs::{FatType, FileSystem, FsOptions};

use super::esp;

/// Below 65525 clusters `fatfs` silently formats FAT16, which `libs/fat32` refuses.
pub const VOLUME_BYTES: usize = 64 * 1024 * 1024;

/// A file small enough to sit in one cluster.
pub const SMALL_NAME: &str = "HELLO.TXT";

/// Larger than one cluster at every size `fatfs` chooses here, so reading it walks the chain.
pub const LARGE_NAME: &str = "BIG.BIN";

pub const LARGE_BYTES: usize = 40 * 1024;

/// Deleted after later live entries exist; its `0xE5` slot tells deleted from end of directory.
pub const DELETED_NAME: &str = "GONE.TXT";

/// A directory, so the walk has somewhere to descend.
pub const DIRECTORY_NAME: &str = "DATA";

pub const NESTED_NAME: &str = "NESTED.TXT";

pub const SMALL_CONTENT: &[u8] = b"the wizard's bedrock reads a disk\n";

pub const NESTED_CONTENT: &[u8] = b"one directory down\n";

/// Byte `index` of [`LARGE_NAME`]; position-dependent, so a read from the wrong cluster shows.
pub const fn large_byte(index: usize) -> u8 {
    ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
}

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
