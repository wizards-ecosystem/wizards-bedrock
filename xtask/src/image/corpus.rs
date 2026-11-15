use std::io::{Read, Seek, Write};

use anyhow::{Context, Result};
use fatfs::{FatType, FileSystem, FsOptions};

use super::esp;

// Below 65,525 clusters `fatfs` formats FAT16, which `libs/fat32` refuses.
pub(crate) const VOLUME_BYTES: usize = 64 * 1024 * 1024;

const SMALL_NAME: &str = "HELLO.TXT";
const LARGE_NAME: &str = "BIG.BIN";
const LARGE_BYTES: usize = 40 * 1024;
const DELETED_NAME: &str = "GONE.TXT";
const DIRECTORY_NAME: &str = "DATA";
const NESTED_NAME: &str = "NESTED.TXT";
const SMALL_CONTENT: &[u8] = b"the wizard's bedrock reads a disk\n";
const NESTED_CONTENT: &[u8] = b"one directory down\n";

const fn large_byte(index: usize) -> u8 {
    ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
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
