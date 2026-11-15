use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Context, Result};
use fscommon::{BufStream, StreamSlice};

use crate::image::corpus;

// The kernel reads the block size from Identify, and QEMU's raw format is 512-byte blocks.
const BLOCK_BYTES: u64 = 512;
const SIGNED_BLOCKS: u64 = 8;
// The FAT32 volume starts where a partition table would put it, so the reader takes a base.
const VOLUME_SECTOR: u64 = 2048;
pub(crate) const NAMESPACE_BYTES: u64 = VOLUME_SECTOR * BLOCK_BYTES + corpus::VOLUME_BYTES as u64;

// Distinct in every block, so a driver that skipped the read or ignored the address fails.
const fn signature(lba: u64) -> u64 {
    0x5749_5A44_0000_0000 | lba
}

pub(crate) fn write_namespace(path: &Path) -> Result<()> {
    let written = (|| -> Result<()> {
        let mut file = crate::paths::create_read_write(path)?;
        file.set_len(NAMESPACE_BYTES)?;
        for lba in 0..SIGNED_BLOCKS {
            file.seek(SeekFrom::Start(lba * BLOCK_BYTES))?;
            file.write_all(&signature(lba).to_le_bytes())?;
        }
        let volume = StreamSlice::new(&mut file, VOLUME_SECTOR * BLOCK_BYTES, NAMESPACE_BYTES)?;
        corpus::populate(BufStream::new(volume))?;
        Ok(())
    })();
    written.with_context(|| format!("writing the NVMe namespace {}", path.display()))
}
