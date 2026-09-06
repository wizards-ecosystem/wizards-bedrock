//! The 512-byte master boot record.
//!
//! UEFI firmware boots an MBR-partitioned disk carrying a single 0xEF partition
//! directly. GPT can come later, when the OS grows its own partitioning story.

use std::io::{Seek, SeekFrom, Write};

use anyhow::{Context, Result, bail};

/// Bytes in a master boot record.
pub const RECORD_SIZE: usize = 512;

/// Offset of the partition table within the record.
const PARTITION_TABLE: usize = 446;

/// Bytes per partition entry.
const ENTRY_SIZE: usize = 16;

/// Entries the table holds.
const ENTRY_COUNT: usize = 4;

/// Offset of the boot signature.
const SIGNATURE_AT: usize = 510;

/// The boot signature itself.
const SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// Partition type code for an EFI System Partition.
const TYPE_EFI_SYSTEM: u8 = 0xEF;

/// The "active" flag. UEFI ignores it; BIOS-era tools still look for it.
const FLAG_BOOTABLE: u8 = 0x80;

/// One entry in the partition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PartitionEntry {
    /// Whether the active flag is set.
    pub bootable: bool,
    /// Partition type code.
    pub kind: u8,
    /// First sector of the partition.
    pub start_lba: u32,
    /// Length of the partition in sectors.
    pub sectors: u32,
}

impl PartitionEntry {
    /// An EFI system partition running from `start_lba` to the end of a disk of
    /// `total_sectors`.
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

    fn to_bytes(self) -> [u8; ENTRY_SIZE] {
        let mut bytes = [0u8; ENTRY_SIZE];
        bytes[0] = if self.bootable { FLAG_BOOTABLE } else { 0 };
        // Bytes 1..4 and 5..8 are CHS addresses, left zero: everything that
        // reads this record uses the LBA fields below.
        bytes[4] = self.kind;
        bytes[8..12].copy_from_slice(&self.start_lba.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.sectors.to_le_bytes());
        bytes
    }
}

/// Lay out a master boot record holding `partitions`.
///
/// # Panics
///
/// Panics if more than four partitions are given — the table has no room.
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

/// Write `record` over sector zero of `disk`, leaving everything after it alone.
pub fn write<D: Write + Seek>(disk: &mut D, record: &[u8; RECORD_SIZE]) -> Result<()> {
    Seek::seek(disk, SeekFrom::Start(0)).context("seeking to the master boot record")?;
    Write::write_all(disk, record).context("writing the master boot record")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const TOTAL_SECTORS: u64 = 64 * 1024 * 1024 / 512;
    const START_LBA: u64 = 2048;

    fn esp_record() -> [u8; RECORD_SIZE] {
        record(&[PartitionEntry::efi_system(START_LBA, TOTAL_SECTORS).unwrap()])
    }

    #[test]
    fn record_is_byte_exact() {
        let mbr = esp_record();
        assert!(mbr[..PARTITION_TABLE].iter().all(|&byte| byte == 0));
        assert_eq!(mbr[446], FLAG_BOOTABLE);
        assert_eq!(&mbr[447..450], &[0, 0, 0]);
        assert_eq!(mbr[450], TYPE_EFI_SYSTEM);
        assert_eq!(u32::from_le_bytes(mbr[454..458].try_into().unwrap()), 2048);
        assert_eq!(
            u32::from_le_bytes(mbr[458..462].try_into().unwrap()),
            129_024
        );
        assert!(mbr[462..SIGNATURE_AT].iter().all(|&byte| byte == 0));
        assert_eq!(&mbr[SIGNATURE_AT..], &SIGNATURE);
    }

    #[test]
    fn an_empty_table_still_carries_the_signature() {
        let mbr = record(&[]);
        assert!(mbr[..SIGNATURE_AT].iter().all(|&byte| byte == 0));
        assert_eq!(&mbr[SIGNATURE_AT..], &SIGNATURE);
    }

    #[test]
    fn entries_round_trip_through_bytes() {
        let entry = PartitionEntry {
            bootable: false,
            kind: 0x0C,
            start_lba: 1,
            sectors: 2,
        };
        let bytes = entry.to_bytes();
        assert_eq!(bytes[0], 0);
        assert_eq!(bytes[4], 0x0C);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 2);
    }

    #[test]
    fn disks_beyond_the_u32_lba_limit_are_rejected() {
        assert!(PartitionEntry::efi_system(START_LBA, 1 << 33).is_err());
    }

    #[test]
    fn a_partition_starting_past_the_disk_is_rejected() {
        assert!(PartitionEntry::efi_system(TOTAL_SECTORS, TOTAL_SECTORS).is_err());
    }

    #[test]
    fn writing_replaces_only_sector_zero() {
        let mut disk = Cursor::new(vec![0xFF; RECORD_SIZE * 2]);
        disk.set_position(4096);
        write(&mut disk, &esp_record()).unwrap();

        let bytes = disk.into_inner();
        assert_eq!(&bytes[..RECORD_SIZE], &esp_record());
        assert!(bytes[RECORD_SIZE..].iter().all(|&byte| byte == 0xFF));
    }
}
