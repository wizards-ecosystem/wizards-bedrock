//! Where everything lives.
//!
//! Every path the build touches is derived here from a handful of constants, so
//! renaming a package or switching profile is a one-line change rather than a
//! hunt for string literals.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::cargo::Profile;

fn collect_directory_entries<T>(
    entries: impl Iterator<Item = std::io::Result<T>>,
) -> std::io::Result<Vec<T>> {
    entries.collect()
}

fn collect_classified_entries<T>(entries: impl Iterator<Item = Result<T>>) -> Result<Vec<T>> {
    entries.collect()
}

const fn repository_entry_is_supported(
    is_symlink: bool,
    is_reparse_point: bool,
    is_dir: bool,
    is_file: bool,
) -> bool {
    !is_symlink && !is_reparse_point && (is_dir || is_file)
}

fn metadata_is_supported(metadata: &std::fs::Metadata) -> bool {
    let file_type = metadata.file_type();
    repository_entry_is_supported(
        file_type.is_symlink(),
        is_reparse_point(metadata),
        file_type.is_dir(),
        file_type.is_file(),
    )
}

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
const fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// One repository entry with its fallible file type already resolved.
#[derive(Debug)]
pub(crate) struct DirectoryEntry {
    entry: std::fs::DirEntry,
    file_type: std::fs::FileType,
}

impl DirectoryEntry {
    /// The entry's full path.
    pub(crate) fn path(&self) -> PathBuf {
        self.entry.path()
    }

    /// Whether the captured entry is a directory.
    pub(crate) fn is_dir(&self) -> bool {
        self.file_type.is_dir()
    }
}

fn classify_directory_entry(entry: std::fs::DirEntry) -> Result<DirectoryEntry> {
    let path = entry.path();
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("classifying {}", path.display()))?;
    let file_type = metadata.file_type();
    ensure!(
        metadata_is_supported(&metadata),
        "repository proof walk refuses redirected or special entry {}",
        path.display()
    );
    Ok(DirectoryEntry { entry, file_type })
}

/// Read and classify a complete directory or refuse its repository proof.
pub(crate) fn directory_entries(directory: &Path) -> Result<Vec<DirectoryEntry>> {
    let metadata = std::fs::symlink_metadata(directory)
        .with_context(|| format!("classifying proof directory {}", directory.display()))?;
    ensure!(
        metadata.file_type().is_dir() && metadata_is_supported(&metadata),
        "repository proof walk refuses redirected or non-directory root {}",
        directory.display()
    );
    let entries =
        std::fs::read_dir(directory).with_context(|| format!("listing {}", directory.display()))?;
    let entries = collect_directory_entries(entries)
        .with_context(|| format!("reading every entry in {}", directory.display()))?;
    collect_classified_entries(entries.into_iter().map(classify_directory_entry))
}

/// Cargo package producing the kernel binary.
pub const KERNEL_PACKAGE: &str = "wizard-kernel";

/// Packages that only ever build for the kernel target.
pub const KERNEL_PACKAGES: &[&str] = &["wost", KERNEL_PACKAGE];

/// Packages that build and run their tests on the host. `libs/` crates are
/// `no_std` but compile for the host under `cfg(test)`, which is the whole
/// point of putting pure logic there.
pub const HOST_PACKAGES: &[&str] = &[
    "acpi-tables",
    "eevdf",
    "elf64",
    "fat32",
    "frame-space",
    "handle-space",
    "memory-map",
    "mmio-range",
    "nvme",
    "panic-record",
    "path-walk",
    "pci-config",
    "pixel",
    "port-space",
    "ps2-keys",
    "register-bits",
    "ring-proto",
    "slab-layout",
    "stack-layout",
    "timebase",
    "trap-cause",
    "vtd",
    "wizzy-handoff",
    "x86-apic",
    "x86-cpuid",
    "x86-paging",
    "x86-pit",
    "x86-tables",
    "xtask",
];

/// The `default-members` entries of a workspace manifest, as package names.
///
/// This list exists twice — here as [`HOST_PACKAGES`], and in the root
/// manifest, where it scopes bare cargo commands to the half that builds for
/// the host. Two copies of a list is two chances to add a crate to one of
/// them, which is exactly what happened to `port-space`: the gate silently
/// stopped covering it. A test below reads the real manifest through this
/// function and requires the two to agree.
///
/// Hand-rolled rather than a TOML dependency, on the `sha256.rs` precedent —
/// one array, one call site. Entries are directory paths and the package name
/// is the last segment, a convention the same test enforces by comparing.
pub fn default_members(manifest: &str) -> Vec<String> {
    manifest
        .split_once("default-members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| {
            list.lines()
                .map(|line| core::iter::Iterator::next(&mut line.split('#')).unwrap_or(line))
                .filter_map(|line| line.split('"').nth(1))
                .filter_map(|path| core::iter::Iterator::next(&mut path.rsplit('/')))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Triple the kernel is cross-compiled for.
pub const KERNEL_TARGET: &str = "x86_64-unknown-none";

/// Cargo feature that compiles the kernel's boot-time self-checks in.
pub const SELFTEST_FEATURE: &str = "selftest";

/// The same feature spelled the way a workspace-scoped `--features` wants it.
///
/// Written out rather than formatted, because a `const` cannot be — so a test
/// below asserts it against the two constants it is made of.
pub const SELFTEST_FEATURE_SPEC: &str = "wizard-kernel/selftest";

/// Cargo feature which makes the production PCI configuration claim refuse.
pub const REFUSE_PCI_FEATURE: &str = "refuse-pci";

/// Workspace-scoped spelling of [`REFUSE_PCI_FEATURE`].
pub const REFUSE_PCI_FEATURE_SPEC: &str = "wizard-kernel/refuse-pci";

/// Name the kernel takes on the EFI system partition. Must match the `path:`
/// line in `boot/limine.conf`.
pub const KERNEL_ON_ESP: &str = "wizard-kernel.elf";

/// The user program, alongside the kernel, handed over as a Limine module.
pub const PROGRAM_ON_ESP: &str = "hello.elf";

/// File name of the bootable disk image.
pub const IMAGE_NAME: &str = "bedrock.img";

/// Per-run QEMU state whose lifetime is shorter than a build artifact.
pub(crate) const QEMU_RUN_DIRECTORY: &str = "qemu runs,private";

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

    /// The TLA+ tools' checksum listing.
    pub fn tla_checksums(&self) -> PathBuf {
        self.root.join("tools").join("tla").join("CHECKSUMS")
    }

    /// The independently generated FAT32 corpus's checksum listing.
    pub fn fat32_corpus_checksums(&self) -> PathBuf {
        self.root
            .join("tools")
            .join("fat32-corpus")
            .join("CHECKSUMS")
    }
}

#[cfg(test)]
mod tests;
