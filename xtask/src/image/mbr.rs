//! The master boot record: UEFI firmware boots a single 0xEF MBR partition directly.

use std::io::{Seek, SeekFrom, Write};

use anyhow::{Context, Result, bail};

pub const RECORD_SIZE: usize = 512;

const PARTITION_TABLE: usize = 446;

const ENTRY_SIZE: usize = 16;

const ENTRY_COUNT: usize = 4;

const SIGNATURE_AT: usize = 510;

const SIGNATURE: [u8; 2] = [0x55, 0xAA];

const TYPE_EFI_SYSTEM: u8 = 0xEF;

/// UEFI ignores the active flag; BIOS-era tools still look for it.
const FLAG_BOOTABLE: u8 = 0x80;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct PartitionEntry {
    pub bootable: bool,
    pub kind: u8,
    pub start_lba: u32,
    pub sectors: u32,
}

impl PartitionEntry {
    /// An EFI system partition from `start_lba` to the end of a `total_sectors` disk.
    pub fn efi_system(start_lba: u64, total_sectors: u64) -> Result<Self> {
        if start_lba >= total_sectors {
            bail!("partition starts at LBA {start_lba} on a disk of only {total_sectors} sectors");
        }
        Ok(Self {
            bootable: true,
            kind: TYPE_EFI_SYSTEM,
            start_lba: u32::try_from(start_lba)
                .with_context(|| format!("start LBA {start_lba} does not fit an MBR entry"))?,
            sectors: u32::try_from(total_sectors - start_lba).with_context(|| {
                format!("a {total_sectors}-sector disk does not fit an MBR entry")
            })?,
        })
    }

    /// The CHS bytes 1..4 and 5..8 stay zero: every reader of this record uses the LBA fields.
    fn to_bytes(self) -> [u8; ENTRY_SIZE] {
        let mut bytes = [0u8; ENTRY_SIZE];
        bytes[0] = if self.bootable { FLAG_BOOTABLE } else { 0 };
        bytes[4] = self.kind;
        bytes[8..12].copy_from_slice(&self.start_lba.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.sectors.to_le_bytes());
        bytes
    }
}

/// A record holding `partitions`; panics past the four the table has room for.
pub fn record(partitions: &[PartitionEntry]) -> [u8; RECORD_SIZE] {
    assert!(
        partitions.len() <= ENTRY_COUNT,
        "an MBR holds at most {ENTRY_COUNT} partitions"
    );

    let mut record = [0u8; RECORD_SIZE];
    for (index, partition) in partitions.iter().enumerate() {
        let at = PARTITION_TABLE + index * ENTRY_SIZE;
        record[at..at + ENTRY_SIZE].copy_from_slice(&partition.to_bytes());
    }
    record[SIGNATURE_AT..].copy_from_slice(&SIGNATURE);
    record
}

/// Sector zero only; everything after it is left alone.
pub fn write<D: Write + Seek>(disk: &mut D, record: &[u8; RECORD_SIZE]) -> Result<()> {
    Seek::seek(disk, SeekFrom::Start(0)).context("seeking to the master boot record")?;
    Write::write_all(disk, record).context("writing the master boot record")
}

#[cfg(test)]
mod tests;
