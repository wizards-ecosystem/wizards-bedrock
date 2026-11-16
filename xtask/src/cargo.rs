use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::paths::{self, Paths};

pub(crate) const VIRTUAL_SOURCE_ROOT: &str = "/wizard/source";
const VIRTUAL_TOOLCHAIN_ROOT: &str = "/wizard/toolchain";
const VIRTUAL_CARGO_ROOT: &str = "/wizard/cargo";

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Profile {
    Dev,
    #[default]
    Release,
}

impl Profile {
    pub(crate) fn directory(self) -> &'static str {
        match self {
            Self::Dev => "debug",
            Self::Release => "release",
        }
    }

    fn flags(self) -> &'static [&'static str] {
        match self {
            Self::Dev => &[],
            Self::Release => &["--release"],
        }
    }
}

pub(crate) fn run<I, S>(paths: &Paths, description: &str, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = command(paths)
        .args(args)
        .status()
        .with_context(|| format!("could not spawn cargo to {description}"))?;
    ensure!(status.success(), "{description} failed ({status})");
    Ok(())
}

fn command(paths: &Paths) -> Command {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(paths.root())
        .env("CARGO_TARGET_DIR", paths.target_dir());
    command
}

pub(crate) fn package_args(packages: &[&str]) -> Vec<String> {
    packages
        .iter()
        .flat_map(|name| ["--package".to_string(), (*name).to_string()])
        .collect()
}

pub(crate) fn build_kernel(paths: &Paths, profile: Profile, features: &[&str]) -> Result<Vec<u8>> {
    println!("[xtask] building kernel ({})", profile.directory());
    let mut args = vec!["build".to_string(), "--locked".to_string()];
    args.extend(package_args(&[paths::KERNEL_PACKAGE]));
    args.extend(["--target".to_string(), paths::KERNEL_TARGET.to_string()]);
    args.extend(profile.flags().iter().map(|flag| (*flag).to_string()));
    if !features.is_empty() {
        args.extend(["--features".to_string(), features.join(",")]);
    }
    args.extend(["--config".to_string(), target_config(paths, &[])?]);
    run(paths, "kernel build", &args)?;

    let elf = paths.kernel_elf(profile);
    let bytes = std::fs::read(&elf).with_context(|| format!("reading {}", elf.display()))?;
    crate::elf::verify(&bytes)
        .with_context(|| format!("{} is not a bootable kernel", elf.display()))?;
    Ok(bytes)
}

// Always release: one witness serves both kernel profiles, and its stack is a single page.
pub(crate) fn build_witness(paths: &Paths) -> Result<Vec<u8>> {
    println!("[xtask] building the ring 3 witness");
    let mut args = vec!["build".to_string(), "--locked".to_string()];
    args.extend(package_args(&[paths::WITNESS_PACKAGE]));
    args.extend(["--target".to_string(), paths::KERNEL_TARGET.to_string()]);
    args.push("--release".to_string());
    // The appended model wins; fat LTO cannot merge it with the sysroot's kernel-model `core`.
    args.extend([
        "--config".to_string(),
        target_config(paths, &["-C", "code-model=small"])?,
        "--config".to_string(),
        "profile.release.lto=false".to_string(),
    ]);
    let status = command(paths)
        .env("CARGO_TARGET_DIR", paths.witness_target_dir())
        .args(&args)
        .status()
        .context("could not spawn cargo to build the ring 3 witness")?;
    ensure!(
        status.success(),
        "the ring 3 witness build failed ({status})"
    );

    let elf = paths.witness_elf();
    std::fs::read(&elf).with_context(|| format!("reading {}", elf.display()))
}

// `--config` appends to the rustflags array in .cargo/config.toml; RUSTFLAGS would replace it.
fn target_config(paths: &Paths, extra: &[&str]) -> Result<String> {
    let mut flags: Vec<String> = extra.iter().map(|flag| toml_string(flag)).collect();
    for (root, virtual_root) in [
        (paths.root().to_path_buf(), VIRTUAL_SOURCE_ROOT),
        (toolchain_root()?, VIRTUAL_TOOLCHAIN_ROOT),
        (cargo_home()?, VIRTUAL_CARGO_ROOT),
    ] {
        let root = root
            .to_str()
            .with_context(|| format!("remap root is not UTF-8: {}", root.display()))?;
        flags.push(toml_string("--remap-path-prefix"));
        flags.push(toml_string(&format!("{root}={virtual_root}")));
    }
    Ok(format!(
        "target.{}.rustflags=[{}]",
        paths::KERNEL_TARGET,
        flags.join(", ")
    ))
}

fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toolchain_root() -> Result<PathBuf> {
    Path::new(env!("CARGO"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .context("the cargo executable is outside a toolchain root")
}

fn cargo_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Ok(PathBuf::from(home));
    }
    let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(home)
        .map(|directory| PathBuf::from(directory).join(".cargo"))
        .with_context(|| format!("{home} is unset, so the Cargo home is unknown"))
}
