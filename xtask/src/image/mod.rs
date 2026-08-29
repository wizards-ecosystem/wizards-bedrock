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

use anyhow::{Context, Result, bail};
use fatfs::{FatType, FileSystem, FsOptions, ReadWriteSeek};
use fscommon::{BufStream, StreamSlice};

use crate::cargo::{self, Profile};
use crate::paths::{self, Paths};

/// Which kernel a normal boot image carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        let signature = crate::qemu::SELFTEST_VERDICT_PREFIX.as_bytes();
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
        let directory = target.join(paths::QEMU_RUN_DIRECTORY);
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;

        for _ in 0..1024 {
            let sequence = NEXT_RUN_IMAGE.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!("wizard-os-{}-{sequence}.img", std::process::id()));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
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

    std::fs::create_dir_all(paths.target_dir())
        .with_context(|| format!("creating {}", paths.target_dir().display()))?;
    println!("[xtask] creating disk image {}", image.display());

    let mut disk = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&image)
        .with_context(|| format!("creating {}", image.display()))?;
    assemble(paths, wanted, &kernel, &image, &mut disk)?;
    Ok(image)
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
    std::fs::read(&kernel).with_context(|| format!("reading {} kernel artifact", wanted.label()))
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
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;

    use super::*;

    const TEST_IMAGE_BYTES: usize = 8 * 1024 * 1024;
    static NEXT_TEST_TARGET: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn two_qemu_images_are_private_and_removed() {
        let target = Arc::new(std::env::temp_dir().join(format!(
            "wizard image,fixture-{}-{}",
            std::process::id(),
            NEXT_TEST_TARGET.fetch_add(1, Ordering::Relaxed)
        )));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let reserve = |target: Arc<PathBuf>, barrier: Arc<std::sync::Barrier>| {
            std::thread::spawn(move || {
                barrier.wait();
                RunImage::reserve(&target).expect("private QEMU image")
            })
        };
        let first = reserve(Arc::clone(&target), Arc::clone(&barrier));
        let second = reserve(Arc::clone(&target), Arc::clone(&barrier));
        barrier.wait();
        let (first, first_file) = first.join().expect("first image reservation");
        let (second, second_file) = second.join().expect("second image reservation");

        assert_ne!(first.path(), second.path());
        assert!(first.path().exists());
        assert!(second.path().exists());
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();
        drop(first_file);
        drop(second_file);
        drop(first);
        drop(second);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
        let _ = std::fs::remove_dir_all(target.as_ref());
    }

    fn image_with(kernel: &[u8]) -> (Cursor<Vec<u8>>, mbr::PartitionEntry) {
        let mut disk = Cursor::new(vec![0u8; TEST_IMAGE_BYTES]);
        let total_sectors = TEST_IMAGE_BYTES as u64 / paths::SECTOR_SIZE;
        let partition = mbr::PartitionEntry::efi_system(paths::PART_START_LBA, total_sectors)
            .expect("a test partition");
        mbr::write(&mut disk, &mbr::record(&[partition])).expect("a test MBR");

        let start = u64::from(partition.start_lba) * paths::SECTOR_SIZE;
        let end = start + u64::from(partition.sectors) * paths::SECTOR_SIZE;
        let slice = StreamSlice::new(&mut disk, start, end).expect("the test partition");
        let mut volume = BufStream::new(slice);
        esp::format(&mut volume, FatType::Fat16).expect("a test filesystem");
        let filesystem = FileSystem::new(volume, FsOptions::new()).expect("mounting the test ESP");
        {
            let boot = filesystem.root_dir().create_dir("boot").expect("/boot");
            esp::write_file(&boot, paths::KERNEL_ON_ESP, kernel).expect("the test kernel");
            esp::write_file(&boot, paths::PROGRAM_ON_ESP, b"not the kernel")
                .expect("the other boot payload");
        }
        filesystem.unmount().expect("flushing the test ESP");
        (disk, partition)
    }

    #[test]
    fn selectors_build_only_their_named_kernel() {
        assert_eq!(Kernel::Shipped.features(), [] as [&str; 0]);
        assert_eq!(Kernel::SelfTest.features(), [paths::SELFTEST_FEATURE]);
    }

    #[test]
    fn both_kernel_kinds_are_read_back_and_identified() {
        let selftest = [
            b"\x7fELF self-test kernel ".as_slice(),
            crate::qemu::SELFTEST_VERDICT_PREFIX.as_bytes(),
        ]
        .concat();
        for (wanted, expected) in [
            (Kernel::Shipped, b"\x7fELF shipped kernel".to_vec()),
            (Kernel::SelfTest, selftest),
        ] {
            let (mut disk, partition) = image_with(&expected);
            let carried = read_kernel(&mut disk, partition).expect("the kernel in the image");
            assert_eq!(carried, expected);
            assert_eq!(Kernel::identify(&carried), wanted);
        }
    }
}
