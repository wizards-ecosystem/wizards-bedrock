//! Driving cargo.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use crate::paths::{self, Paths};

pub(crate) const VIRTUAL_SOURCE_ROOT: &str = "/wizard/source";
const VIRTUAL_TOOLCHAIN_ROOT: &str = "/wizard/toolchain";
const VIRTUAL_CARGO_ROOT: &str = "/wizard/cargo";
const ENCODED_FLAG_SEPARATOR: char = '\u{1f}';

/// Which cargo profile to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// Unoptimised, for quick iteration.
    Dev,
    /// Optimised, and what a real image ships.
    #[default]
    Release,
}

impl Profile {
    /// Directory cargo writes this profile's artifacts to.
    pub fn directory(self) -> &'static str {
        match self {
            Self::Dev => "debug",
            Self::Release => "release",
        }
    }

    /// Flags selecting this profile on a cargo command line.
    pub fn flags(self) -> &'static [&'static str] {
        match self {
            Self::Dev => &[],
            Self::Release => &["--release"],
        }
    }
}

/// Run a cargo command, failing if it does not exit cleanly.
pub fn run<I, S>(paths: &Paths, description: &str, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = command(paths)
        .args(args)
        .status()
        .with_context(|| format!("could not spawn cargo to {description}"))?;
    if !status.success() {
        bail!("{description} failed ({status})");
    }
    Ok(())
}

fn command(paths: &Paths) -> Command {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(paths.root())
        .env("CARGO_TARGET_DIR", child_process_path(paths.target_dir()));
    command
}

fn kernel_command(paths: &Paths) -> Result<Command> {
    kernel_command_with_roots(paths, active_kernel_remap_roots(paths)?)
}

fn kernel_command_with_roots(paths: &Paths, roots: [PathBuf; 3]) -> Result<Command> {
    let [source_root, toolchain_root, cargo_root] = roots;
    let tool_bin = toolchain_root.join("bin");
    let executable = |name: &str| {
        tool_bin.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        })
    };
    let inherited = std::env::var_os("PATH").context("PATH is unavailable")?;
    let path = std::env::join_paths(
        std::iter::once(tool_bin.clone()).chain(std::env::split_paths(&inherited)),
    )
    .context("constructing pinned kernel compiler PATH")?;
    let mut command = command(paths);
    command
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            kernel_encoded_rustflags(&source_root, &toolchain_root, &cargo_root)?,
        )
        .env("RUSTC", executable("rustc"))
        .env("RUSTDOC", executable("rustdoc"))
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env("PATH", path);
    Ok(command)
}

pub(crate) fn active_kernel_remap_roots(paths: &Paths) -> Result<[PathBuf; 3]> {
    Ok([
        crate::mutation_workspace::canonical_existing_real_directory(
            paths.root(),
            "kernel source root",
        )?,
        active_toolchain_root()?,
        active_cargo_home()?,
    ])
}

pub(crate) fn kernel_encoded_rustflags(
    source_root: &Path,
    toolchain_root: &Path,
    cargo_root: &Path,
) -> Result<OsString> {
    let mappings = [
        (source_root, VIRTUAL_SOURCE_ROOT),
        (toolchain_root, VIRTUAL_TOOLCHAIN_ROOT),
        (cargo_root, VIRTUAL_CARGO_ROOT),
    ];
    let mut distinct = std::collections::BTreeSet::new();
    let mut flags = vec![
        "-C".to_string(),
        "code-model=kernel".to_string(),
        "-C".to_string(),
        "relocation-model=static".to_string(),
    ];
    for (root, virtual_root) in mappings {
        let root = child_process_path(root);
        let root = root
            .to_str()
            .with_context(|| format!("kernel remap root is not UTF-8: {}", root.display()))?;
        ensure!(
            !root.is_empty()
                && !root.contains('=')
                && !root.contains(ENCODED_FLAG_SEPARATOR)
                && !root.chars().any(char::is_control),
            "kernel remap root cannot be encoded safely: {root:?}"
        );
        let folded = if cfg!(windows) {
            root.to_ascii_lowercase()
        } else {
            root.to_string()
        };
        ensure!(
            distinct.insert(folded),
            "kernel remap roots must be distinct"
        );
        flags.push("--remap-path-prefix".to_string());
        flags.push(format!("{root}={virtual_root}"));
    }
    Ok(OsString::from(
        flags.join(&ENCODED_FLAG_SEPARATOR.to_string()),
    ))
}

fn active_toolchain_root() -> Result<PathBuf> {
    let cargo = std::fs::canonicalize(env!("CARGO"))
        .with_context(|| format!("canonicalizing Cargo executable {}", env!("CARGO")))?;
    let root = cargo
        .parent()
        .and_then(Path::parent)
        .context("Cargo executable is outside a toolchain root")?;
    crate::mutation_workspace::canonical_existing_real_directory(
        root,
        "kernel compiler toolchain root",
    )
}

fn active_cargo_home() -> Result<PathBuf> {
    let configured = std::env::var_os("CARGO_HOME").filter(|value| !value.is_empty());
    let home = match configured {
        Some(home) => PathBuf::from(home),
        None => platform_home()?.join(".cargo"),
    };
    crate::mutation_workspace::canonical_existing_real_directory(&home, "kernel Cargo home")
}

#[cfg(windows)]
fn platform_home() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("USERPROFILE is unavailable")
}

#[cfg(not(windows))]
fn platform_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("HOME is unavailable")
}

/// Return an equivalent path that command-line toolchains can consume.
pub(crate) fn child_process_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::path::{Component, Prefix};

        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return path.to_path_buf();
        };
        let mut ordinary = match prefix.kind() {
            Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
            Prefix::VerbatimUNC(server, share) => {
                let mut root = OsString::from(r"\\");
                root.push(server);
                root.push(r"\");
                root.push(share);
                PathBuf::from(root)
            }
            _ => return path.to_path_buf(),
        };
        for component in components {
            if !matches!(component, Component::RootDir) {
                ordinary.push(component.as_os_str());
            }
        }
        ordinary
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// Expand `packages` into the `--package NAME` pairs cargo expects.
pub fn package_args(packages: &[&str]) -> Vec<String> {
    packages
        .iter()
        .flat_map(|name| ["--package".to_string(), (*name).to_string()])
        .collect()
}

/// Compile the kernel with `features` enabled, returning the ELF it produced.
pub fn build_kernel(paths: &Paths, profile: Profile, features: &[&str]) -> Result<PathBuf> {
    println!("[xtask] building kernel ({})", profile.directory());

    let args = kernel_build_args(profile, features);
    let status = kernel_command(paths)?
        .args(&args)
        .status()
        .context("could not spawn cargo to kernel build")?;
    if !status.success() {
        bail!("kernel build failed ({status})");
    }

    let elf = paths.kernel_elf(profile);
    if !elf.exists() {
        bail!(
            "kernel build reported success but produced no {}",
            elf.display()
        );
    }
    // A linker script fails quietly (an unnamed section is placed, not
    // refused), so what it produced is checked against what it promised.
    crate::elf::verify_file(&elf)?;
    Ok(elf)
}

fn kernel_build_args(profile: Profile, features: &[&str]) -> Vec<String> {
    let mut args = vec!["build".to_string(), "--locked".to_string()];
    args.extend(package_args(&[paths::KERNEL_PACKAGE]));
    args.extend(["--target".to_string(), paths::KERNEL_TARGET.to_string()]);
    args.extend(profile.flags().iter().map(|flag| (*flag).to_string()));
    if !features.is_empty() {
        args.extend(["--features".to_string(), features.join(",")]);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_is_the_default_profile() {
        assert_eq!(Profile::default(), Profile::Release);
    }

    #[test]
    fn package_args_pairs_each_name_with_a_flag() {
        assert_eq!(
            package_args(&["wost", "wizard-kernel"]),
            ["--package", "wost", "--package", "wizard-kernel"]
        );
    }

    #[test]
    fn kernel_builds_are_locked_to_the_recorded_dependencies() {
        for args in [
            kernel_build_args(Profile::Dev, &[]),
            kernel_build_args(Profile::Release, &[paths::SELFTEST_FEATURE]),
        ] {
            assert_eq!(
                args.iter().filter(|arg| arg.as_str() == "--locked").count(),
                1
            );
        }
    }

    #[test]
    fn a_cargo_command_uses_the_paths_private_target() {
        let paths = Paths::new(
            PathBuf::from("workspace"),
            Some(std::path::Path::new("private-build")),
        );
        let command = command(&paths);
        assert_eq!(command.get_current_dir(), Some(paths.root()));
        assert!(command.get_envs().any(|(name, value)| {
            name == "CARGO_TARGET_DIR" && value == Some(paths.target_dir().as_os_str())
        }));
    }

    #[test]
    fn kernel_builds_use_the_exact_codegen_and_virtual_path_contract() {
        let source = Path::new(r"C:\source checkout");
        let toolchain = Path::new(r"C:\rust toolchain");
        let cargo_home = Path::new(r"C:\cargo home");
        let encoded = kernel_encoded_rustflags(source, toolchain, cargo_home).unwrap();
        assert_eq!(
            encoded,
            OsString::from(
                "-C\u{1f}code-model=kernel\u{1f}-C\u{1f}relocation-model=static\u{1f}\
                 --remap-path-prefix\u{1f}C:\\source checkout=/wizard/source\u{1f}\
                 --remap-path-prefix\u{1f}C:\\rust toolchain=/wizard/toolchain\u{1f}\
                 --remap-path-prefix\u{1f}C:\\cargo home=/wizard/cargo"
            )
        );
        assert!(kernel_encoded_rustflags(source, source, cargo_home).is_err());
        assert!(kernel_encoded_rustflags(Path::new("bad=root"), toolchain, cargo_home).is_err());

        let paths = Paths::new(source.to_path_buf(), Some(Path::new(r"C:\target")));
        let command = kernel_command_with_roots(
            &paths,
            [
                source.to_path_buf(),
                toolchain.to_path_buf(),
                cargo_home.to_path_buf(),
            ],
        )
        .unwrap();
        for (name, suffix) in [("RUSTC", "rustc"), ("RUSTDOC", "rustdoc")] {
            assert!(command.get_envs().any(|(observed, value)| {
                observed == name
                    && value.is_some_and(|value| {
                        value.to_string_lossy().contains("rust toolchain")
                            && value.to_string_lossy().contains(suffix)
                    })
            }));
        }
        for wrapper in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
            assert!(
                command.get_envs().any(|(observed, value)| {
                    observed == wrapper && value == Some(OsStr::new(""))
                })
            );
        }
        let path = command
            .get_envs()
            .find_map(|(name, value)| (name == "PATH").then_some(value).flatten())
            .unwrap();
        assert_eq!(
            std::env::split_paths(path).next().as_deref(),
            Some(Path::new(r"C:\rust toolchain\bin"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_cargo_command_sheds_the_windows_verbatim_target_prefix() {
        let paths = Paths::new(
            PathBuf::from(r"\\?\C:\repo"),
            Some(std::path::Path::new(r"\\?\C:\repo\private-build")),
        );
        let command = command(&paths);
        assert!(command.get_envs().any(|(name, value)| {
            name == "CARGO_TARGET_DIR"
                && value == Some(std::ffi::OsStr::new(r"C:\repo\private-build"))
        }));
    }

    #[cfg(windows)]
    #[test]
    fn a_cargo_command_sheds_the_windows_verbatim_unc_target_prefix() {
        let paths = Paths::new(
            PathBuf::from(r"\\?\UNC\server\share\repo"),
            Some(std::path::Path::new(
                r"\\?\UNC\server\share\repo\private-build",
            )),
        );
        let command = command(&paths);
        assert!(command.get_envs().any(|(name, value)| {
            name == "CARGO_TARGET_DIR"
                && value == Some(std::ffi::OsStr::new(r"\\server\share\repo\private-build"))
        }));
    }
}
