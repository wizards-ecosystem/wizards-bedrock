//! Driving cargo.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
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
pub fn run<I, S>(root: &Path, description: &str, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(env!("CARGO"))
        .current_dir(root)
        .args(args)
        .status()
        .with_context(|| format!("could not spawn cargo to {description}"))?;
    if !status.success() {
        bail!("{description} failed ({status})");
    }
    Ok(())
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

    let mut args = vec!["build".to_string()];
    args.extend(package_args(&[paths::KERNEL_PACKAGE]));
    args.extend(["--target".to_string(), paths::KERNEL_TARGET.to_string()]);
    args.extend(profile.flags().iter().map(|flag| (*flag).to_string()));
    if !features.is_empty() {
        args.extend(["--features".to_string(), features.join(",")]);
    }
    run(paths.root(), "kernel build", &args)?;

    let elf = paths.kernel_elf(profile);
    if !elf.exists() {
        bail!(
            "kernel build reported success but produced no {}",
            elf.display()
        );
    }
    // A linker script goes wrong quietly — an unnamed section is placed by a
    // heuristic rather than refused — so what it was written to produce is
    // checked against what it produced, every time.
    crate::elf::verify_file(&elf)?;
    Ok(elf)
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
}
