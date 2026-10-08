//! The EFI system partition is built in memory, so its layout is testable
//! without a kernel build or a 64 MiB image.
//!
//! FAT16 rather than FAT32 here: FAT32 needs at least 65525 clusters, which
//! would mean a ~33 MiB allocation per test.

use std::io::Cursor;

use fatfs::{FatType, FileSystem, FsOptions};
use xtask::image::esp;

const VOLUME_BYTES: usize = 16 * 1024 * 1024;

#[test]
fn files_written_to_the_volume_read_back() {
    use std::io::Read;

    let mut volume = Cursor::new(vec![0u8; VOLUME_BYTES]);
    esp::format(&mut volume, FatType::Fat16).expect("format");

    let payload = b"the wizard's bedrock";
    {
        let filesystem = FileSystem::new(&mut volume, FsOptions::new()).expect("mount");
        {
            let boot = filesystem
                .root_dir()
                .create_dir("boot")
                .expect("create /boot");
            esp::write_file(&boot, "wizard-kernel.elf", payload).expect("write kernel");
        }
        filesystem.unmount().expect("unmount");
    }

    volume.set_position(0);
    let filesystem = FileSystem::new(&mut volume, FsOptions::new()).expect("remount");
    let mut file = filesystem
        .root_dir()
        .open_dir("boot")
        .expect("open /boot")
        .open_file("wizard-kernel.elf")
        .expect("open kernel");

    let mut read_back = Vec::new();
    file.read_to_end(&mut read_back).expect("read kernel");
    assert_eq!(read_back, payload);
}

#[test]
fn the_volume_carries_the_wizard_label() {
    let mut volume = Cursor::new(vec![0u8; VOLUME_BYTES]);
    esp::format(&mut volume, FatType::Fat16).expect("format");

    let filesystem = FileSystem::new(&mut volume, FsOptions::new()).expect("mount");
    assert_eq!(filesystem.volume_label().trim(), "WIZARD-ESP");
}
