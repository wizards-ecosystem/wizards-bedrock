//! Building the bootable disk image.
//!
//! Everything happens in-process in pure Rust — no xorriso, no mtools. The
//! artifact is a raw disk with an MBR partition table holding one FAT32 EFI
//! system partition.

pub mod corpus;
pub mod esp;
pub mod mbr;
pub mod program;

#[cfg(test)]
mod foreign;

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail, ensure};
use fatfs::{FatType, FileSystem, FsOptions, ReadWriteSeek};
use fscommon::{BufStream, StreamSlice};

use crate::cargo::{self, Profile};
use crate::paths::{self, Paths};

/// Which kernel a normal boot image carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kernel {
    /// The kernel that boots for a user.
    Shipped,
    /// The kernel with its boot-time checks and verdict device compiled in.
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
        let signature = crate::qemu::SELFTEST_IMAGE_MARKER.as_bytes();
        if bytes
            .windows(signature.len())
            .any(|window| window == signature)
        {
            Self::SelfTest
        } else {
            Self::Shipped
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Shipped => "shipped",
            Self::SelfTest => "self-test",
        }
    }
}

/// Compile `kernel` and assemble a bootable image around it.
pub fn build(paths: &Paths, profile: Profile, kernel: Kernel) -> Result<PathBuf> {
    build_with_features(paths, profile, kernel, kernel.features())
}

/// A boot image owned by one QEMU run.
pub(crate) struct RunImage {
    path: PathBuf,
}

impl RunImage {
    fn reserve(target: &Path) -> Result<(Self, File)> {
        let directory = qemu_run_directory(target)?;

        for _ in 0..1024 {
            let sequence = NEXT_RUN_IMAGE.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!("bedrock-{}-{sequence}.img", std::process::id()));
            let mut options = OpenOptions::new();
            OpenOptions::read(&mut options, true);
            OpenOptions::write(&mut options, true);
            OpenOptions::create_new(&mut options, true);
            match OpenOptions::open(&options, &path) {
                Ok(file) => return Ok((Self { path }, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("creating {}", path.display()));
                }
            }
        }
        bail!("could not reserve a private QEMU boot image")
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RunImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

static NEXT_RUN_IMAGE: AtomicU64 = AtomicU64::new(0);
static NEXT_IMAGE_STAGE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn qemu_run_directory(target: &Path) -> Result<PathBuf> {
    crate::mutate::workspace::prepare_real_directory_tree(
        &target.join(paths::QEMU_RUN_DIRECTORY),
        "QEMU private run directory",
    )
}

/// Compile `kernel` into an image that one QEMU process owns and removes.
pub(crate) fn build_for_run(paths: &Paths, profile: Profile, kernel: Kernel) -> Result<RunImage> {
    build_for_run_with_features(paths, profile, kernel, kernel.features())
}

/// Build the fixed degraded kernel into a private QEMU image.
pub(crate) fn build_degraded_for_run(paths: &Paths, profile: Profile) -> Result<RunImage> {
    build_for_run_with_features(
        paths,
        profile,
        Kernel::Shipped,
        &[paths::REFUSE_PCI_FEATURE],
    )
}

fn build_with_features(
    paths: &Paths,
    profile: Profile,
    wanted: Kernel,
    features: &[&str],
) -> Result<PathBuf> {
    let kernel = kernel_bytes(paths, profile, wanted, features)?;
    let image = paths.image();

    println!("[xtask] creating disk image {}", image.display());
    publish_image(paths.target_dir(), &image, |staging, disk| {
        assemble(paths, wanted, &kernel, staging, disk)
    })?;
    Ok(image)
}

fn publish_image(
    parent: &Path,
    destination: &Path,
    build: impl FnOnce(&Path, &mut File) -> Result<()>,
) -> Result<()> {
    ensure!(
        destination == parent.join(paths::IMAGE_NAME),
        "boot image is not the exact configured target child"
    );
    let canonical_parent = crate::mutate::workspace::prepare_real_directory_tree(
        parent,
        "boot-image target directory",
    )?;
    let published_path = canonical_parent.join(paths::IMAGE_NAME);
    crate::mutate::workspace::require_direct_file_if_present(
        &canonical_parent,
        &published_path,
        "existing boot image",
    )?;

    let sequence = NEXT_IMAGE_STAGE.fetch_add(1, Ordering::Relaxed);
    let staging = canonical_parent.join(format!(
        ".{}-{}-{sequence}.incomplete",
        paths::IMAGE_NAME,
        std::process::id()
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        OpenOptions::read(&mut options, true);
        OpenOptions::write(&mut options, true);
        OpenOptions::create_new(&mut options, true);
        let mut disk = OpenOptions::open(&options, &staging)
            .with_context(|| format!("creating boot-image stage {}", staging.display()))?;
        build(&staging, &mut disk)?;
        disk.sync_all()
            .with_context(|| format!("syncing boot-image stage {}", staging.display()))?;
        drop(disk);

        crate::mutate::workspace::require_direct_file(
            &canonical_parent,
            &staging,
            "boot-image stage",
        )?;
        ensure!(
            crate::mutate::workspace::canonical_existing_real_directory(
                parent,
                "boot-image target directory",
            )? == canonical_parent,
            "boot-image target directory changed before publication"
        );
        let staged = crate::sha256::digest_reader(crate::mutate::workspace::open_real_file(
            &staging,
            "boot-image stage",
        )?)
        .with_context(|| format!("hashing boot-image stage {}", staging.display()))?;

        crate::mutate::workspace::publish_staged_file(
            &canonical_parent,
            &staging,
            &published_path,
            "boot image",
            |published| {
                let digest = crate::sha256::digest_reader(
                    crate::mutate::workspace::open_real_file(published, "boot image")?,
                )
                .with_context(|| format!("hashing boot image {}", destination.display()))?;
                ensure!(
                    digest == staged,
                    "published boot image differs from its complete stage"
                );
                Ok(())
            },
        )?;
        Ok(())
    })();
    crate::mutate::workspace::finish_staged_operation(
        &canonical_parent,
        &staging,
        "boot-image stage",
        result,
    )
}

fn build_for_run_with_features(
    paths: &Paths,
    profile: Profile,
    wanted: Kernel,
    features: &[&str],
) -> Result<RunImage> {
    let kernel = kernel_bytes(paths, profile, wanted, features)?;
    let (image, mut disk) = RunImage::reserve(paths.target_dir())?;
    println!(
        "[xtask] creating private disk image {}",
        image.path.display()
    );
    let result = assemble(paths, wanted, &kernel, image.path(), &mut disk);
    drop(disk);
    result?;
    Ok(image)
}

fn kernel_bytes(
    paths: &Paths,
    profile: Profile,
    wanted: Kernel,
    features: &[&str],
) -> Result<Vec<u8>> {
    let kernel = cargo::build_kernel(paths, profile, features)?;
    crate::elf::read_file(&kernel)
        .with_context(|| format!("reading {} kernel artifact", wanted.label()))
}

fn assemble(
    paths: &Paths,
    wanted: Kernel,
    kernel: &[u8],
    image: &Path,
    disk: &mut File,
) -> Result<()> {
    let bytes = paths::DISK_MIB * 1024 * 1024;
    let total_sectors = bytes / paths::SECTOR_SIZE;
    disk.set_len(bytes)
        .with_context(|| format!("sizing {} to {bytes} bytes", image.display()))?;

    let partition = mbr::PartitionEntry::efi_system(paths::PART_START_LBA, total_sectors)?;
    mbr::write(&mut *disk, &mbr::record(&[partition]))?;

    let start = u64::from(partition.start_lba) * paths::SECTOR_SIZE;
    let end = start + u64::from(partition.sectors) * paths::SECTOR_SIZE;
    let slice = StreamSlice::new(&mut *disk, start, end)
        .context("addressing the EFI system partition within the image")?;
    let mut volume = BufStream::new(slice);

    esp::format(&mut volume, FatType::Fat32)?;
    let filesystem =
        FileSystem::new(volume, FsOptions::new()).context("mounting the EFI system partition")?;
    esp::populate(&filesystem, paths, kernel)?;
    filesystem
        .unmount()
        .context("flushing the EFI system partition")?;

    let carried = read_kernel(&mut *disk, partition)?;
    if carried != kernel {
        bail!(
            "the kernel read back from {} differs from the verified {} artifact",
            image.display(),
            wanted.label()
        );
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

fn read_kernel<D: ReadWriteSeek>(disk: &mut D, partition: mbr::PartitionEntry) -> Result<Vec<u8>> {
    let start = u64::from(partition.start_lba) * paths::SECTOR_SIZE;
    let end = start + u64::from(partition.sectors) * paths::SECTOR_SIZE;
    let slice = StreamSlice::new(disk, start, end)
        .context("addressing the EFI system partition while reading it back")?;
    let volume = BufStream::new(slice);
    let filesystem = FileSystem::new(volume, FsOptions::new())
        .context("mounting the EFI system partition while reading it back")?;
    let mut bytes = Vec::new();
    {
        let root = filesystem.root_dir();
        let boot = root
            .open_dir("boot")
            .context("opening /boot while reading the image back")?;
        let mut kernel = boot
            .open_file(paths::KERNEL_ON_ESP)
            .context("opening the kernel while reading the image back")?;
        kernel
            .read_to_end(&mut bytes)
            .context("reading the kernel back from the image")?;
    }
    filesystem
        .unmount()
        .context("unmounting the read-back EFI system partition")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
