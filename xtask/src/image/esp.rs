use std::path::Path;

use anyhow::{Context, Result};
use fatfs::{Dir, FatType, FileSystem, FormatVolumeOptions, ReadWriteSeek};

use crate::paths::{self, Paths};

const MAX_LIMINE_CONFIG_BYTES: u64 = 64 * 1024;

pub fn format<D: ReadWriteSeek>(volume: &mut D, fat_type: FatType) -> Result<()> {
    let options = FormatVolumeOptions::new()
        .fat_type(fat_type)
        .volume_label(paths::ESP_LABEL);
    fatfs::format_volume(volume, options).context("formatting the EFI system partition")
}

pub fn write_file<D: ReadWriteSeek>(dir: &Dir<'_, D>, name: &str, data: &[u8]) -> Result<()> {
    use std::io::Write;

    let mut file = dir
        .create_file(name)
        .with_context(|| format!("creating {name}"))?;
    file.truncate()
        .with_context(|| format!("truncating {name}"))?;
    file.write_all(data)
        .with_context(|| format!("writing {name}"))
}

fn read_source(source: &Path, maximum: u64, label: &str) -> Result<Vec<u8>> {
    crate::qualification::read_bounded_file(source, maximum, label)
        .with_context(|| format!("reading {label} {}", source.display()))
}

fn copy_into<D: ReadWriteSeek>(
    dir: &Dir<'_, D>,
    name: &str,
    source: &Path,
    maximum: u64,
    label: &str,
) -> Result<()> {
    let data = read_source(source, maximum, label)?;
    write_file(dir, name, &data)
}

fn copy_limine_efi<D: ReadWriteSeek>(dir: &Dir<'_, D>, paths: &Paths) -> Result<()> {
    let source = paths.limine_efi();
    let data = read_source(
        &source,
        crate::vendor::MAX_PAYLOAD_BYTES,
        "Limine EFI image",
    )?;
    crate::vendor::verify_payload_bytes(&paths.limine_checksums(), &source, &data)?;
    write_file(dir, "BOOTX64.EFI", &data)
}

/// The program sits in /boot beside the kernel: Limine's `module_path` is volume-relative.
pub fn populate<D: ReadWriteSeek>(
    filesystem: &FileSystem<D>,
    paths: &Paths,
    kernel: &[u8],
) -> Result<()> {
    let root = filesystem.root_dir();

    let efi_boot = root
        .create_dir("EFI")
        .and_then(|efi| efi.create_dir("BOOT"))
        .context("creating /EFI/BOOT")?;
    copy_limine_efi(&efi_boot, paths)?;

    let boot = root.create_dir("boot").context("creating /boot")?;
    write_file(&boot, paths::KERNEL_ON_ESP, kernel)?;

    let program = super::program::build().context("building the fixed ring-3 witness")?;
    write_file(&boot, paths::PROGRAM_ON_ESP, &program)?;

    let limine = boot.create_dir("limine").context("creating /boot/limine")?;
    copy_into(
        &limine,
        "limine.conf",
        &paths.limine_conf(),
        MAX_LIMINE_CONFIG_BYTES,
        "Limine configuration",
    )
}

#[cfg(test)]
mod tests;
