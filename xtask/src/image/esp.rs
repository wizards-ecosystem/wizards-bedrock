use std::io::Write;

use anyhow::{Context, Result};
use fatfs::{Dir, FatType, FileSystem, FormatVolumeOptions, ReadWriteSeek};

use crate::paths::{self, Paths};

pub(crate) fn format<D: ReadWriteSeek>(volume: &mut D, fat_type: FatType) -> Result<()> {
    let options = FormatVolumeOptions::new()
        .fat_type(fat_type)
        .volume_label(paths::ESP_LABEL);
    fatfs::format_volume(volume, options).context("formatting a FAT volume")
}

pub(crate) fn write_file<D: ReadWriteSeek>(
    directory: &Dir<'_, D>,
    name: &str,
    data: &[u8],
) -> Result<()> {
    let mut file = directory
        .create_file(name)
        .with_context(|| format!("creating {name}"))?;
    file.truncate()
        .with_context(|| format!("truncating {name}"))?;
    file.write_all(data)
        .with_context(|| format!("writing {name}"))
}

// The witness sits in /boot beside the kernel because Limine's `module_path` is volume-relative.
pub(crate) fn populate<D: ReadWriteSeek>(
    filesystem: &FileSystem<D>,
    paths: &Paths,
    kernel: &[u8],
    witness: &[u8],
) -> Result<()> {
    let root = filesystem.root_dir();
    let efi_boot = root
        .create_dir("EFI")
        .and_then(|efi| efi.create_dir("BOOT"))
        .context("creating /EFI/BOOT")?;
    let limine = crate::vendor::read_verified(&paths.limine_checksums(), &paths.limine_efi())?;
    write_file(&efi_boot, "BOOTX64.EFI", &limine)?;

    let boot = root.create_dir("boot").context("creating /boot")?;
    write_file(&boot, paths::KERNEL_ON_ESP, kernel)?;
    write_file(&boot, paths::WITNESS_ON_ESP, witness)?;

    let configuration = paths.limine_conf();
    let configuration = std::fs::read(&configuration)
        .with_context(|| format!("reading {}", configuration.display()))?;
    let limine = boot.create_dir("limine").context("creating /boot/limine")?;
    write_file(&limine, "limine.conf", &configuration)
}
