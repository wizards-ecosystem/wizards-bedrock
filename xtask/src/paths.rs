use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

use crate::cargo::Profile;

pub(crate) const KERNEL_PACKAGE: &str = "wizard-kernel";
pub(crate) const KERNEL_PACKAGES: &[&str] = &["wost", KERNEL_PACKAGE];
pub(crate) const KERNEL_TARGET: &str = "x86_64-unknown-none";
pub(crate) const SELFTEST_FEATURE: &str = "selftest";
pub(crate) const SELFTEST_FEATURE_SPEC: &str = "wizard-kernel/selftest";
// Must match the `path:` line in boot/limine.conf.
pub(crate) const KERNEL_ON_ESP: &str = "wizard-kernel.elf";
pub(crate) const PROGRAM_ON_ESP: &str = "hello.elf";
pub(crate) const IMAGE_NAME: &str = "bedrock.img";
pub(crate) const NVME_IMAGE_NAME: &str = "nvme.img";
pub(crate) const DISK_MIB: u64 = 64;
pub(crate) const SECTOR_SIZE: u64 = 512;
pub(crate) const PART_START_LBA: u64 = 2048;
pub(crate) const ESP_LABEL: [u8; 11] = *b"WIZARD-ESP ";
const QEMU_RUNS: &str = "qemu-runs";
const VM_DIRECTORY: &str = "vm";

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Paths {
    root: PathBuf,
    target_dir: PathBuf,
}

impl Paths {
    // `build.target-dir` in a cargo config is not honoured; set CARGO_TARGET_DIR instead.
    pub(crate) fn discover() -> Result<Self> {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .with_context(|| format!("no workspace above {}", manifest_dir.display()))?
            .to_path_buf();
        let target_dir = match std::env::var_os("CARGO_TARGET_DIR") {
            Some(directory) => root.join(directory),
            None => root.join("target"),
        };
        Ok(Self { root, target_dir })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn target_dir(&self) -> &Path {
        &self.target_dir
    }

    pub(crate) fn kernel_elf(&self, profile: Profile) -> PathBuf {
        self.target_dir
            .join(KERNEL_TARGET)
            .join(profile.directory())
            .join(KERNEL_PACKAGE)
    }

    pub(crate) fn image(&self) -> PathBuf {
        self.target_dir.join(IMAGE_NAME)
    }

    pub(crate) fn vm_dir(&self) -> PathBuf {
        self.root.join(VM_DIRECTORY)
    }

    pub(crate) fn limine_efi(&self) -> PathBuf {
        self.root.join("boot").join("limine").join("BOOTX64.EFI")
    }

    pub(crate) fn limine_conf(&self) -> PathBuf {
        self.root.join("boot").join("limine.conf")
    }

    pub(crate) fn limine_checksums(&self) -> PathBuf {
        self.root.join("boot").join("limine").join("CHECKSUMS")
    }

    pub(crate) fn tla_checksums(&self) -> PathBuf {
        self.root.join("tools").join("tla").join("CHECKSUMS")
    }

    pub(crate) fn tla_jar(&self) -> PathBuf {
        self.root.join("tools").join("tla").join("tla2tools.jar")
    }
}

pub(crate) fn create_read_write(path: &Path) -> std::io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) struct RunDirectory {
    path: PathBuf,
}

impl RunDirectory {
    pub(crate) fn create(paths: &Paths) -> Result<Self> {
        let sequence = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
        let path = paths
            .target_dir
            .join(QEMU_RUNS)
            .join(format!("{}-{sequence}", std::process::id()));
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing stale {}", path.display()))?;
        }
        std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn image(&self) -> PathBuf {
        self.path.join(IMAGE_NAME)
    }
}

impl Drop for RunDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
