//! The check, format, and housekeeping tasks.
//!
//! Each cargo-driven task runs twice — once for the host packages, once for the
//! cross-compiled ones — reading both lists from [`crate::paths`], so a local
//! run and a CI run cannot drift apart.

use std::time::Duration;

use anyhow::{Context, Result};

use crate::cargo::{self, Profile};
use crate::cli::{CleanScope, FmtMode};
use crate::paths::{self, Paths};
use crate::{image, qemu, vendor};

/// Type-check every crate.
pub fn check(paths: &Paths, message_format: Option<&str>) -> Result<()> {
    let extra: Vec<String> = message_format
        .map(|format| vec![format!("--message-format={format}")])
        .unwrap_or_default();
    for_each_half(paths, "check", "check", &extra, &[])
}

/// Lint every crate, treating warnings as errors.
///
/// The `-D warnings` goes after `--` on purpose: setting `RUSTFLAGS` instead
/// would replace the target rustflags in `.cargo/config.toml` and silently
/// build a kernel without `code-model=kernel`.
pub fn clippy(paths: &Paths) -> Result<()> {
    for_each_half(
        paths,
        "clippy",
        "clippy",
        &[],
        &["-D".to_string(), "warnings".to_string()],
    )
}

/// Format the workspace, or report what is unformatted.
pub fn fmt(paths: &Paths, mode: FmtMode) -> Result<()> {
    let mut args = vec!["fmt".to_string(), "--all".to_string()];
    if mode == FmtMode::Check {
        args.extend(["--".to_string(), "--check".to_string()]);
    }
    cargo::run(paths.root(), "formatting", &args)
}

/// Run the host test suite. The kernel crates have no host tests by
/// construction — see `docs/005-code-conventions.md`.
pub fn test(paths: &Paths) -> Result<()> {
    let mut args = vec!["test".to_string()];
    args.extend(cargo::package_args(paths::HOST_PACKAGES));
    cargo::run(paths.root(), "tests", &args)
}

/// Boot a self-test kernel under QEMU and require its verdict.
///
/// This is the only automated check that exercises the kernel on a machine
/// rather than as arithmetic on the host.
pub fn test_boot(paths: &Paths, profile: Profile) -> Result<()> {
    let image = image::build(paths, profile, &[qemu::SELFTEST_FEATURE])?;
    qemu::run_selftest(paths, &image, SELFTEST_TIMEOUT)
}

/// Generous enough for a cold QEMU start on a loaded machine; short enough that
/// a hung kernel is reported rather than waited on.
const SELFTEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Everything the gate runs, in the order that fails fastest.
pub fn ci(paths: &Paths) -> Result<()> {
    fmt(paths, FmtMode::Check)?;
    clippy(paths)?;
    test(paths)?;
    vendor::verify(paths)?;
    test_boot(paths, Profile::Release)?;
    image::build(paths, Profile::Release, &[])?;
    println!("[xtask] ci passed");
    Ok(())
}

/// Remove build output.
pub fn clean(paths: &Paths, scope: CleanScope) -> Result<()> {
    cargo::run(paths.root(), "clean", ["clean"])?;
    if scope == CleanScope::Everything {
        let vm_dir = paths.vm_dir();
        if vm_dir.exists() {
            std::fs::remove_dir_all(&vm_dir)
                .with_context(|| format!("removing {}", vm_dir.display()))?;
            println!("[xtask] removed {}", vm_dir.display());
        }
    }
    Ok(())
}

/// Run `subcommand` over the host packages and then the kernel packages.
fn for_each_half(
    paths: &Paths,
    description: &str,
    subcommand: &str,
    extra: &[String],
    after_dashes: &[String],
) -> Result<()> {
    for (packages, target) in [
        (paths::HOST_PACKAGES, None),
        (paths::KERNEL_PACKAGES, Some(paths::KERNEL_TARGET)),
    ] {
        let args = half_args(subcommand, packages, target, extra, after_dashes);
        let half = target.unwrap_or("host");
        cargo::run(paths.root(), &format!("{description} ({half})"), &args)?;
    }
    Ok(())
}

/// The command line for one half of the workspace.
///
/// `--all-targets` goes to the host half only. The kernel crates have no test
/// or bench targets and cannot have any, but the host crates do — and without
/// it neither `check` nor `clippy` ever sees their test code. A `useless_vec`
/// in `sha256.rs`'s tests sat there unreported until someone ran clippy by hand.
fn half_args(
    subcommand: &str,
    packages: &[&str],
    target: Option<&str>,
    extra: &[String],
    after_dashes: &[String],
) -> Vec<String> {
    let mut args = vec![subcommand.to_string()];
    args.extend(cargo::package_args(packages));
    match target {
        Some(target) => args.extend(["--target".to_string(), target.to_string()]),
        None => args.push("--all-targets".to_string()),
    }
    args.extend_from_slice(extra);
    if !after_dashes.is_empty() {
        args.push("--".to_string());
        args.extend_from_slice(after_dashes);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(target: Option<&str>) -> Vec<String> {
        half_args("clippy", &["pixel"], target, &[], &["-D".to_string()])
    }

    #[test]
    fn the_host_half_lints_its_tests() {
        assert_eq!(
            args(None),
            ["clippy", "--package", "pixel", "--all-targets", "--", "-D"]
        );
    }

    #[test]
    fn the_kernel_half_is_cross_compiled_and_has_no_extra_targets() {
        let args = args(Some(paths::KERNEL_TARGET));
        assert!(args.contains(&"--target".to_string()));
        assert!(
            !args.contains(&"--all-targets".to_string()),
            "the kernel crates have no host test targets"
        );
    }

    #[test]
    fn nothing_follows_a_bare_double_dash() {
        let args = half_args("check", &["pixel"], None, &[], &[]);
        assert!(!args.contains(&"--".to_string()));
    }
}
