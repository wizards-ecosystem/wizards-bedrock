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
