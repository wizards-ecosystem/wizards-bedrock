//! Where everything lives.
//!
//! Every path the build touches is derived here from a handful of constants, so
//! renaming a package or switching profile is a one-line change rather than a
//! hunt for string literals.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cargo::Profile;

/// Cargo package producing the kernel binary.
pub const KERNEL_PACKAGE: &str = "wizard-kernel";

/// Packages that only ever build for the kernel target.
pub const KERNEL_PACKAGES: &[&str] = &["wost", KERNEL_PACKAGE];

/// Packages that build and run their tests on the host. `libs/` crates are
/// `no_std` but compile for the host under `cfg(test)`, which is the whole
/// point of putting pure logic there.
pub const HOST_PACKAGES: &[&str] = &["pixel", "x86-apic", "x86-tables", "xtask"];

/// Triple the kernel is cross-compiled for.
pub const KERNEL_TARGET: &str = "x86_64-unknown-none";

/// Name the kernel takes on the EFI system partition. Must match the `path:`
/// line in `boot/limine.conf`.
pub const KERNEL_ON_ESP: &str = "wizard-kernel.elf";

/// File name of the bootable disk image.
pub const IMAGE_NAME: &str = "wizard-os.img";

/// Size of that image, in MiB.
pub const DISK_MIB: u64 = 64;

/// Bytes per logical sector.
pub const SECTOR_SIZE: u64 = 512;

/// First partition begins 1 MiB in — the conventional alignment.
pub const PART_START_LBA: u64 = 2048;

/// FAT volume label: exactly eleven bytes, space padded.
pub const ESP_LABEL: [u8; 11] = *b"WIZARD-ESP ";

/// Resolved locations for one checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
    target_dir: PathBuf,
}

impl Paths {
    /// Derive every path from an explicit root, touching no environment, so
    /// tests can drive it.
    pub fn new(root: PathBuf, target_dir_override: Option<&Path>) -> Self {
        let target_dir = match target_dir_override {
            Some(dir) if dir.is_absolute() => dir.to_path_buf(),
            Some(dir) => root.join(dir),
            None => root.join("target"),
        };
        Self { root, target_dir }
    }

    /// Locate the workspace from cargo's environment.
    ///
    /// `build.target-dir` in a cargo config is deliberately not honoured —
    /// reading it would mean parsing `cargo metadata`. Set `CARGO_TARGET_DIR`
    /// instead; see `docs/005-code-conventions.md`.
    pub fn discover() -> Result<Self> {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .with_context(|| format!("no workspace above {}", manifest_dir.display()))?
            .to_path_buf();
        let override_dir = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from);
        Ok(Self::new(root, override_dir.as_deref()))
    }

    /// Workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory cargo writes build artifacts to.
    pub fn target_dir(&self) -> &Path {
        &self.target_dir
    }

    /// The compiled kernel ELF for `profile`.
    pub fn kernel_elf(&self, profile: Profile) -> PathBuf {
        self.target_dir
            .join(KERNEL_TARGET)
            .join(profile.directory())
            .join(KERNEL_PACKAGE)
    }

    /// The bootable disk image.
    pub fn image(&self) -> PathBuf {
        self.target_dir.join(IMAGE_NAME)
    }

    /// Writable copy of the UEFI variable store QEMU boots against.
    pub fn efi_vars(&self) -> PathBuf {
        self.target_dir.join("efi-vars.fd")
    }

    /// Generated VMware VM directory.
    pub fn vm_dir(&self) -> PathBuf {
        self.root.join("vm")
    }

    /// Vendored Limine UEFI executable.
    pub fn limine_efi(&self) -> PathBuf {
        self.root.join("boot").join("limine").join("BOOTX64.EFI")
    }

    /// Limine's boot menu configuration.
    pub fn limine_conf(&self) -> PathBuf {
        self.root.join("boot").join("limine.conf")
    }

    /// Recorded digests of everything vendored under `boot/`.
    pub fn limine_checksums(&self) -> PathBuf {
        self.root.join("boot").join("limine").join("CHECKSUMS")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(target_dir: Option<&str>) -> Paths {
        Paths::new(PathBuf::from("/repo"), target_dir.map(Path::new))
    }

    #[test]
    fn kernel_elf_derives_from_profile() {
        let paths = paths(None);
        assert!(
            paths
                .kernel_elf(Profile::Release)
                .ends_with("x86_64-unknown-none/release/wizard-kernel")
        );
        assert!(
            paths
                .kernel_elf(Profile::Dev)
                .ends_with("x86_64-unknown-none/debug/wizard-kernel")
        );
    }

    #[test]
    fn target_dir_defaults_below_root() {
        assert_eq!(paths(None).target_dir(), Path::new("/repo").join("target"));
    }

    #[test]
    fn relative_target_dir_is_joined_to_root() {
        assert_eq!(
            paths(Some("build")).target_dir(),
            Path::new("/repo").join("build")
        );
    }

    #[test]
    fn absolute_target_dir_is_used_verbatim() {
        let absolute = if cfg!(windows) {
            r"C:\shared\target"
        } else {
            "/shared/target"
        };
        assert_eq!(paths(Some(absolute)).target_dir(), Path::new(absolute));
    }

    #[test]
    fn image_lives_in_the_target_dir() {
        assert_eq!(
            paths(Some("build")).image(),
            Path::new("/repo").join("build").join(IMAGE_NAME)
        );
    }
}
