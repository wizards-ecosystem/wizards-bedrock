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
    crate::mutation_workspace::prepare_real_directory_tree(
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
    let canonical_parent = crate::mutation_workspace::prepare_real_directory_tree(
        parent,
        "boot-image target directory",
    )?;
    let published_path = canonical_parent.join(paths::IMAGE_NAME);
    crate::mutation_workspace::require_direct_file_if_present(
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

        crate::mutation_workspace::require_direct_file(
            &canonical_parent,
            &staging,
            "boot-image stage",
        )?;
        ensure!(
            crate::mutation_workspace::canonical_existing_real_directory(
                parent,
                "boot-image target directory",
            )? == canonical_parent,
            "boot-image target directory changed before publication"
        );
        let staged = crate::sha256::digest_reader(crate::mutation_workspace::open_real_file(
            &staging,
            "boot-image stage",
        )?)
        .with_context(|| format!("hashing boot-image stage {}", staging.display()))?;

        crate::mutation_workspace::publish_staged_file(
            &canonical_parent,
            &staging,
            &published_path,
            "boot image",
            |published| {
                let digest = crate::sha256::digest_reader(
                    crate::mutation_workspace::open_real_file(published, "boot image")?,
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
    crate::mutation_workspace::finish_staged_operation(
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
    use std::io::{Cursor, Write};
    use std::sync::Arc;

    use super::*;

    const TEST_IMAGE_BYTES: usize = 8 * 1024 * 1024;
    static NEXT_TEST_TARGET: AtomicU64 = AtomicU64::new(0);

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating image directory symlink");
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_dir(target, link).expect("creating image directory symlink");
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating image file symlink");
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("creating image file symlink");
    }

    #[cfg(unix)]
    fn remove_directory_symlink(path: &Path) {
        std::fs::remove_file(path).expect("removing image directory symlink");
    }

    #[cfg(windows)]
    fn remove_directory_symlink(path: &Path) {
        std::fs::remove_dir(path).expect("removing image directory symlink");
    }

    fn test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "wizard-image-{label}-{}-{}",
            std::process::id(),
            NEXT_TEST_TARGET.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("image test root");
        root
    }

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

    #[test]
    fn private_qemu_images_refuse_a_redirected_run_root_before_writing() {
        let root = test_root("redirected-run");
        let target = root.join("target");
        let outside = root.join("outside");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"outside authority").unwrap();
        let run_root = target.join(paths::QEMU_RUN_DIRECTORY);
        create_directory_symlink(&outside, &run_root);

        assert!(RunImage::reserve(&target).is_err());
        assert_eq!(
            crate::paths::directory_entries(&outside).unwrap().len(),
            1,
            "private image creation escaped its configured target"
        );
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"outside authority"
        );
        remove_directory_symlink(&run_root);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authoritative_images_refuse_a_redirected_target_before_writing() {
        let root = test_root("publication-parent-link");
        let target = root.join("target");
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"outside authority").unwrap();
        create_directory_symlink(&outside, &target);
        let image = target.join(paths::IMAGE_NAME);
        let builder_ran = std::cell::Cell::new(false);

        assert!(
            publish_image(&target, &image, |_path, file| {
                builder_ran.set(true);
                file.write_all(b"forged image")
                    .context("writing forged image")
            })
            .is_err()
        );
        assert!(!builder_ran.get(), "boot-image staging escaped its target");
        assert_eq!(crate::paths::directory_entries(&outside).unwrap().len(), 1);
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"outside authority"
        );
        remove_directory_symlink(&target);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authoritative_image_publication_never_writes_through_an_external_link() {
        let root = test_root("publication-link");
        let target = root.join("target");
        let outside = root.join("outside");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let sentinel = outside.join("sentinel");
        let image = target.join(paths::IMAGE_NAME);
        std::fs::write(&sentinel, b"outside authority").unwrap();

        std::fs::hard_link(&sentinel, &image).unwrap();
        publish_image(&target, &image, |_path, file| {
            file.write_all(b"complete image")
                .context("writing image fixture")
        })
        .unwrap();
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        assert_eq!(std::fs::read(&image).unwrap(), b"complete image");

        std::fs::remove_file(&image).unwrap();
        create_file_symlink(&sentinel, &image);
        assert!(
            publish_image(&target, &image, |_path, file| {
                file.write_all(b"forged").context("writing forged fixture")
            })
            .is_err()
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        std::fs::remove_file(&image).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_authoritative_image_stage_preserves_the_complete_image() {
        let root = test_root("publication-failure");
        let target = root.join("target");
        std::fs::create_dir(&target).unwrap();
        let image = target.join(paths::IMAGE_NAME);
        publish_image(&target, &image, |_path, file| {
            file.write_all(b"old complete image")?;
            Ok(())
        })
        .unwrap();

        let error = publish_image(&target, &image, |_path, file| {
            file.write_all(b"partial")?;
            bail!("injected image construction failure")
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("injected image construction failure")
        );
        assert_eq!(std::fs::read(&image).unwrap(), b"old complete image");
        assert_eq!(crate::paths::directory_entries(&target).unwrap().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
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
