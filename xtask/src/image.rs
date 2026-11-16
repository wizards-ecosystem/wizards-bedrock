pub(crate) mod corpus;
mod esp;
mod mbr;
mod witness;

use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use fatfs::{FatType, FileSystem, FsOptions, ReadWriteSeek};
use fscommon::{BufStream, StreamSlice};

use crate::cargo::{self, Profile};
use crate::paths::{self, Paths};

// Only a self-test kernel prints it, as one literal that the optimiser cannot split.
const SELFTEST_IMAGE_MARKER: &str = "selftest: image marker v1";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kernel {
    Shipped,
    SelfTest,
}

impl Kernel {
    fn features(self) -> &'static [&'static str] {
        match self {
            Self::Shipped => &[],
            Self::SelfTest => &[paths::SELFTEST_FEATURE],
        }
    }

    fn identify(bytes: &[u8]) -> Self {
        let marker = SELFTEST_IMAGE_MARKER.as_bytes();
        if bytes.windows(marker.len()).any(|window| window == marker) {
            Self::SelfTest
        } else {
            Self::Shipped
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Shipped => "shipped",
            Self::SelfTest => "self-test",
        }
    }
}

pub(crate) fn build(
    paths: &Paths,
    profile: Profile,
    kernel: Kernel,
    destination: &Path,
) -> Result<()> {
    let bytes = cargo::build_kernel(paths, profile, kernel.features())?;
    let witness = witness::build(paths)?;
    println!("[xtask] creating disk image {}", destination.display());
    let partial = destination.with_extension("partial");
    let assembled = paths::create_read_write(&partial)
        .with_context(|| format!("creating {}", partial.display()))
        .and_then(|mut disk| {
            assemble(paths, kernel, &bytes, &witness, &mut disk)?;
            disk.sync_all()
                .with_context(|| format!("syncing {}", partial.display()))
        });
    if let Err(error) = assembled {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    std::fs::rename(&partial, destination)
        .with_context(|| format!("publishing {}", destination.display()))
}

fn assemble(
    paths: &Paths,
    wanted: Kernel,
    kernel: &[u8],
    witness: &[u8],
    disk: &mut File,
) -> Result<()> {
    let bytes = paths::DISK_MIB * 1024 * 1024;
    disk.set_len(bytes).context("sizing the disk image")?;
    let partition =
        mbr::PartitionEntry::efi_system(paths::PART_START_LBA, bytes / paths::SECTOR_SIZE)?;
    mbr::write(&mut *disk, &mbr::record(&[partition]))?;

    let mut volume = esp_volume(&mut *disk, partition)?;
    esp::format(&mut volume, FatType::Fat32)?;
    let filesystem =
        FileSystem::new(volume, FsOptions::new()).context("mounting the EFI system partition")?;
    esp::populate(&filesystem, paths, kernel, witness)?;
    filesystem
        .unmount()
        .context("flushing the EFI system partition")?;

    let carried = read_kernel(&mut *disk, partition)?;
    if carried != kernel {
        bail!("the kernel read back from the image differs from the verified artifact");
    }
    let found = Kernel::identify(&carried);
    if found != wanted {
        bail!(
            "requested a {} kernel image, but its payload identifies as {}",
            wanted.label(),
            found.label()
        );
    }
    println!("[xtask] image ready ({} kernel read back)", found.label());
    Ok(())
}

fn esp_volume<D: ReadWriteSeek>(
    disk: D,
    partition: mbr::PartitionEntry,
) -> Result<BufStream<StreamSlice<D>>> {
    let start = u64::from(partition.start_lba) * paths::SECTOR_SIZE;
    let end = start + u64::from(partition.sectors) * paths::SECTOR_SIZE;
    let slice = StreamSlice::new(disk, start, end)
        .context("addressing the EFI system partition within the image")?;
    Ok(BufStream::new(slice))
}

fn read_kernel<D: ReadWriteSeek>(disk: D, partition: mbr::PartitionEntry) -> Result<Vec<u8>> {
    let filesystem = FileSystem::new(esp_volume(disk, partition)?, FsOptions::new())
        .context("mounting the EFI system partition to read it back")?;
    let mut bytes = Vec::new();
    filesystem
        .root_dir()
        .open_dir("boot")
        .and_then(|boot| boot.open_file(paths::KERNEL_ON_ESP))
        .and_then(|mut kernel| kernel.read_to_end(&mut bytes))
        .context("reading the kernel back from the image")?;
    filesystem
        .unmount()
        .context("unmounting the read-back EFI system partition")?;
    Ok(bytes)
}
