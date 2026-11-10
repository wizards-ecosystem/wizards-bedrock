use std::io::{Cursor, Write};
use std::sync::Arc;

use super::*;
use crate::test_support::{create_directory_symlink, create_file_symlink};

const TEST_IMAGE_BYTES: usize = 8 * 1024 * 1024;

fn test_root(label: &str) -> PathBuf {
    crate::test_support::reserve_temp_directory(&format!("wizard-image-{label}"))
}

#[test]
fn two_qemu_images_are_private_and_removed() {
    let target = Arc::new(crate::test_support::reserve_temp_directory(
        "wizard image,fixture",
    ));
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
    crate::test_support::remove_directory_link(&run_root);
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
    crate::test_support::remove_directory_link(&target);
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
        crate::qemu::SELFTEST_IMAGE_MARKER.as_bytes(),
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

#[test]
fn a_formatted_verdict_fragment_cannot_pose_as_a_selftest_image() {
    assert_eq!(
        Kernel::identify(b"\x7fELF selftest: verdict v1 PASS"),
        Kernel::Shipped
    );
    assert_eq!(
        Kernel::identify(b"\x7fELF selftest: image marker v2"),
        Kernel::Shipped
    );
}
