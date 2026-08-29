//! Formatting and populating the EFI system partition.

use std::path::Path;

use anyhow::{Context, Result};
use fatfs::{Dir, FatType, FileSystem, FormatVolumeOptions, ReadWriteSeek};

use crate::paths::{self, Paths};

/// Format `volume` with the wizard label.
pub fn format<D: ReadWriteSeek>(volume: &mut D, fat_type: FatType) -> Result<()> {
    let options = FormatVolumeOptions::new()
        .fat_type(fat_type)
        .volume_label(paths::ESP_LABEL);
    fatfs::format_volume(volume, options).context("formatting the EFI system partition")
}

/// Write `data` into `dir` as `name`.
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

/// Copy the host file at `source` into `dir` as `name`.
pub fn copy_into<D: ReadWriteSeek>(dir: &Dir<'_, D>, name: &str, source: &Path) -> Result<()> {
    let data = std::fs::read(source).with_context(|| format!("reading {}", source.display()))?;
    write_file(dir, name, &data)
}

/// Lay out the boot tree the firmware and Limine expect.
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
    copy_into(&efi_boot, "BOOTX64.EFI", &paths.limine_efi())?;

    let boot = root.create_dir("boot").context("creating /boot")?;
    write_file(&boot, paths::KERNEL_ON_ESP, kernel)?;

    // The initrd, such as it is: one program, built rather than compiled
    // (D-100). It goes beside the kernel because Limine's `module_path` is
    // relative to the boot volume, not to the kernel.
    let program = super::program::build();
    write_file(&boot, paths::PROGRAM_ON_ESP, &program)?;

    let limine = boot.create_dir("limine").context("creating /boot/limine")?;
    copy_into(&limine, "limine.conf", &paths.limine_conf())
}
