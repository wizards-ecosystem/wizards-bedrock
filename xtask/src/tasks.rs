//! The check, format, and housekeeping tasks.
//!
//! Each cargo-driven task runs twice — once for the host packages, once for the
//! cross-compiled ones — reading both lists from [`crate::paths`], so a local
//! run and a CI run cannot drift apart.

use anyhow::{Context, Result};

use crate::cargo::{self, Profile};
use crate::paths::{self, Paths};
use crate::{image, vendor};

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
pub fn fmt(paths: &Paths, check: bool) -> Result<()> {
    let mut args = vec!["fmt".to_string(), "--all".to_string()];
    if check {
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

/// Everything CI runs, in the order that fails fastest.
pub fn ci(paths: &Paths) -> Result<()> {
    fmt(paths, true)?;
    clippy(paths)?;
    test(paths)?;
    vendor::verify(paths)?;
    image::build(paths, Profile::Release)?;
    println!("[xtask] ci passed");
    Ok(())
}

/// Remove build output.
pub fn clean(paths: &Paths, all: bool) -> Result<()> {
    cargo::run(paths.root(), "clean", ["clean"])?;
    if all {
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
        let mut args = vec![subcommand.to_string()];
        args.extend(cargo::package_args(packages));
        if let Some(target) = target {
            args.extend(["--target".to_string(), target.to_string()]);
        }
        args.extend_from_slice(extra);
        if !after_dashes.is_empty() {
            args.push("--".to_string());
            args.extend_from_slice(after_dashes);
        }

        let half = target.unwrap_or("host");
        cargo::run(paths.root(), &format!("{description} ({half})"), &args)?;
    }
    Ok(())
}
