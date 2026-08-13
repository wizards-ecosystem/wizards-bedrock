//! Building the bootable disk image.
//!
//! Everything happens in-process in pure Rust — no xorriso, no mtools. The
//! artifact is a raw disk with an MBR partition table holding one FAT32 EFI
//! system partition.

pub mod esp;
pub mod mbr;

use std::fs::OpenOptions;
use std::path::PathBuf;

use anyhow::{Context, Result};
use fatfs::{FatType, FileSystem, FsOptions};
use fscommon::{BufStream, StreamSlice};

use crate::cargo::{self, Profile};
use crate::paths::{self, Paths};

/// Compile the kernel and assemble a bootable image around it.
pub fn build(paths: &Paths, profile: Profile) -> Result<PathBuf> {
    let kernel = cargo::build_kernel(paths, profile)?;
    let image = paths.image();

    std::fs::create_dir_all(paths.target_dir())
        .with_context(|| format!("creating {}", paths.target_dir().display()))?;
    println!("[xtask] creating disk image {}", image.display());

    let bytes = paths::DISK_MIB * 1024 * 1024;
    let total_sectors = bytes / paths::SECTOR_SIZE;

    let mut disk = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&image)
        .with_context(|| format!("creating {}", image.display()))?;
    disk.set_len(bytes)
        .with_context(|| format!("sizing {} to {bytes} bytes", image.display()))?;

    let partition = mbr::PartitionEntry::efi_system(paths::PART_START_LBA, total_sectors)?;
    mbr::write(&mut disk, &mbr::record(&[partition]))?;

    let start = u64::from(partition.start_lba) * paths::SECTOR_SIZE;
    let end = start + u64::from(partition.sectors) * paths::SECTOR_SIZE;
    let slice = StreamSlice::new(&mut disk, start, end)
        .context("addressing the EFI system partition within the image")?;
    let mut volume = BufStream::new(slice);

    esp::format(&mut volume, FatType::Fat32)?;
    let filesystem =
        FileSystem::new(volume, FsOptions::new()).context("mounting the EFI system partition")?;
    esp::populate(&filesystem, paths, &kernel)?;
    filesystem
        .unmount()
        .context("flushing the EFI system partition")?;

    println!("[xtask] image ready");
    Ok(image)
}
