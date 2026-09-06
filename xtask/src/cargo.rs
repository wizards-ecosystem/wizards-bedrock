//! Driving cargo.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::paths::{self, Paths};

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
        .env("CARGO_TARGET_DIR", paths.target_dir());
    command
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
    run(paths, "kernel build", &args)?;

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
}
