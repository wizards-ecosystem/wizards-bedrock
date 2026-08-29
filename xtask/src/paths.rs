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
    "path-walk",
    "pci-config",
    "pixel",
    "port-space",
    "ps2-keys",
    "ring-proto",
    "slab-layout",
    "stack-layout",
    "timebase",
    "trap-cause",
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
                .map(|line| line.split('#').next().unwrap_or(line))
                .filter_map(|line| line.split('"').nth(1))
                .filter_map(|path| path.rsplit('/').next())
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
pub const IMAGE_NAME: &str = "wizard-os.img";

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
mod tests {
    use super::*;

    fn paths(target_dir: Option<&str>) -> Paths {
        Paths::new(PathBuf::from("/repo"), target_dir.map(Path::new))
    }

    #[test]
    fn default_members_reads_a_list_of_paths_as_package_names() {
        let manifest = "\
[workspace]
default-members = [
    \"libs/pixel\",
    \"libs/x86-apic\",
    \"xtask\",
]
";
        assert_eq!(
            default_members(manifest),
            ["pixel", "x86-apic", "xtask"],
            "a nested path names the crate in its last segment"
        );
    }

    #[test]
    fn default_members_ignores_a_comment_and_a_missing_key() {
        let manifest = "\
default-members = [
    # \"libs/not-a-member\" — commented out, and quoted inside the comment
    \"libs/pixel\",
]
";
        assert_eq!(default_members(manifest), ["pixel"]);
        assert!(default_members("[workspace]\nmembers = [\"a\"]\n").is_empty());
        assert!(default_members("").is_empty());
    }

    /// The list lives here and in the root manifest, and drifted once: a new
    /// `libs/` crate reached `default-members` but not [`HOST_PACKAGES`], so
    /// `cargo xtask test` and `cargo xtask clippy` quietly stopped covering
    /// it while a bare `cargo test` still did. This is the check that says so.
    #[test]
    fn host_packages_are_exactly_the_workspace_default_members() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("root manifest");

        assert_eq!(default_members(&manifest), HOST_PACKAGES);
    }

    /// The spec is two other constants glued together; nothing but this notices
    /// if one of them moves.
    #[test]
    fn feature_specs_name_the_kernel_package_and_their_features() {
        assert_eq!(
            SELFTEST_FEATURE_SPEC,
            format!("{KERNEL_PACKAGE}/{SELFTEST_FEATURE}")
        );
        assert_eq!(
            REFUSE_PCI_FEATURE_SPEC,
            format!("{KERNEL_PACKAGE}/{REFUSE_PCI_FEATURE}")
        );
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

    /// The names here and the paths in `boot/limine.conf` are one contract
    /// written twice.
    ///
    /// `KERNEL_ON_ESP`'s doc says "must match the `path:` line" and nothing
    /// read that file. A drift produces an unbootable image, found forty-five
    /// seconds into `boot-check` rather than by `cargo test` — the same shape
    /// as the `default-members` drift the test above exists for (D-105).
    #[test]
    fn the_boot_configuration_names_the_files_the_image_builder_writes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let config = std::fs::read_to_string(root.join("boot").join("limine.conf"))
            .expect("boot/limine.conf");

        let named = |key: &str| -> String {
            config
                .lines()
                .find_map(|line| line.trim().strip_prefix(key))
                .map(|value| value.trim().rsplit('/').next().unwrap_or("").to_string())
                .unwrap_or_else(|| panic!("no {key} line in boot/limine.conf"))
        };

        assert_eq!(named("path:"), KERNEL_ON_ESP);
        assert_eq!(named("module_path:"), PROGRAM_ON_ESP);
    }

    /// Every crate under `libs/` is in both lists.
    ///
    /// The test above compares the two lists **to each other**, so a crate in
    /// *neither* is symmetric and passes — and it needs no manifest edit to
    /// exist, because Cargo promotes a path dependency inside the workspace
    /// directory to a member on its own. So `libs/foo` depended on by
    /// `kernel/` builds fine while `cargo xtask test`, clippy's host pass and
    /// `doc` never see it. That is the `port-space` drift the sibling test
    /// exists to prevent, entered by the other door (D-124).
    #[test]
    fn every_libs_crate_is_in_both_lists() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("root manifest");
        let members = default_members(&manifest);

        let mut found = Vec::new();
        for entry in std::fs::read_dir(root.join("libs"))
            .expect("libs/")
            .flatten()
        {
            if !entry.path().is_dir() {
                continue;
            }
            // A directory is a crate when it has a manifest; anything else is
            // not this test's business.
            if !entry.path().join("Cargo.toml").exists() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                members.contains(&name),
                "libs/{name} is not in the workspace default-members"
            );
            assert!(
                HOST_PACKAGES.contains(&name.as_str()),
                "libs/{name} is not in HOST_PACKAGES"
            );
            found.push(name);
        }
        assert!(
            found.len() > 15,
            "only {} crates found in libs/",
            found.len()
        );
    }
}
