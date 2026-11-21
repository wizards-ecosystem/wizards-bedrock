use core::time::Duration;

use anyhow::{Context, Result};

use crate::cargo::{self, Profile};
use crate::cli::{CleanScope, FmtMode};
use crate::image::{self, Kernel};
use crate::paths::{self, Paths, RunDirectory};
use crate::{qemu, supply_chain, vendor};

const SELFTEST_QUIET: Duration = Duration::from_secs(24);
// Cargo's `xtask` profile builds the running executable here, and Windows cannot delete it.
const OWN_PROFILE_DIRECTORY: &str = "xtask";
// A debug kernel runs several times slower under emulation.
const DEBUG_SELFTEST_QUIET: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Packages {
    Host,
    Kernel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CargoPass {
    name: &'static str,
    packages: Packages,
    features: Option<&'static str>,
}

const CARGO_PASSES: &[CargoPass] = &[
    CargoPass {
        name: "host",
        packages: Packages::Host,
        features: None,
    },
    CargoPass {
        name: "kernel shipped",
        packages: Packages::Kernel,
        features: None,
    },
    CargoPass {
        name: "kernel self-test",
        packages: Packages::Kernel,
        features: Some(paths::SELFTEST_FEATURE_SPEC),
    },
];

pub(crate) fn ci(paths: &Paths) -> Result<()> {
    fmt(paths, FmtMode::Check)?;
    clippy(paths)?;
    test(paths)?;
    supply_chain::verify(paths)?;
    vendor::verify(paths)?;
    test_boot(paths, Profile::Release)?;
    test_boot(paths, Profile::Dev)?;
    boot_check(paths, Profile::Release)?;
    degraded_check(paths, Profile::Release)?;
    println!("[xtask] ci passed");
    Ok(())
}

pub(crate) fn check(paths: &Paths, message_format: Option<&str>) -> Result<()> {
    let extra: Vec<String> = message_format
        .map(|format| format!("--message-format={format}"))
        .into_iter()
        .collect();
    for_each_pass(paths, "check", &extra)
}

// `-D warnings` goes after `--`: RUSTFLAGS would replace the kernel target's rustflags.
pub(crate) fn clippy(paths: &Paths) -> Result<()> {
    for_each_pass(paths, "clippy", &["--", "-D", "warnings"].map(String::from))
}

pub(crate) fn fmt(paths: &Paths, mode: FmtMode) -> Result<()> {
    let mut args = vec!["fmt", "--all"];
    if mode == FmtMode::Check {
        args.extend(["--", "--check"]);
    }
    cargo::run(paths, "formatting", args)
}

pub(crate) fn test(paths: &Paths) -> Result<()> {
    let mut args = vec!["test".to_string(), "--locked".to_string()];
    args.extend(package_selection(Packages::Host));
    cargo::run(paths, "tests", &args)
}

pub(crate) fn test_boot(paths: &Paths, profile: Profile) -> Result<()> {
    let run = RunDirectory::create(paths)?;
    image::build(paths, profile, Kernel::SelfTest, &run.image())?;
    let (quiet, processors) = match profile {
        Profile::Release => (SELFTEST_QUIET, qemu::Processors::Two),
        Profile::Dev => (DEBUG_SELFTEST_QUIET, qemu::Processors::One),
    };
    qemu::run_selftest(&run, quiet, processors)
}

pub(crate) fn boot_check(paths: &Paths, profile: Profile) -> Result<()> {
    let run = RunDirectory::create(paths)?;
    image::build(paths, profile, Kernel::Shipped, &run.image())?;
    qemu::boot_check(&run)
}

pub(crate) fn degraded_check(paths: &Paths, profile: Profile) -> Result<()> {
    let run = RunDirectory::create(paths)?;
    image::build(paths, profile, Kernel::Shipped, &run.image())?;
    qemu::degraded_check(&run)
}

#[expect(
    clippy::std_instead_of_core,
    reason = "core::io::ErrorKind is unstable"
)]
pub(crate) fn clean(paths: &Paths, scope: CleanScope) -> Result<()> {
    if scope == CleanScope::Everything {
        let vm = paths.vm_dir();
        match std::fs::remove_dir_all(&vm) {
            Ok(()) => println!("[xtask] removed {}", vm.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("removing {}", vm.display())),
        }
    }
    let target = paths.target_dir();
    let entries = match std::fs::read_dir(target) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", target.display())),
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", target.display()))?;
        if entry.file_name() == OWN_PROFILE_DIRECTORY {
            continue;
        }
        let path = entry.path();
        let removed = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        removed.with_context(|| format!("removing {}", path.display()))?;
    }
    println!("[xtask] removed build output under {}", target.display());
    Ok(())
}

fn for_each_pass(paths: &Paths, subcommand: &str, extra: &[String]) -> Result<()> {
    for pass in CARGO_PASSES {
        let mut args = vec![subcommand.to_string(), "--locked".to_string()];
        args.extend(package_selection(pass.packages));
        // `--all-targets` on the host pass only: the kernel crates have no test targets.
        if pass.packages == Packages::Host {
            args.push("--all-targets".to_string());
        } else {
            args.extend(["--target".to_string(), paths::KERNEL_TARGET.to_string()]);
        }
        if let Some(features) = pass.features {
            args.extend(["--features".to_string(), features.to_string()]);
        }
        args.extend_from_slice(extra);
        cargo::run(paths, &format!("{subcommand} ({})", pass.name), &args)?;
    }
    Ok(())
}

fn package_selection(packages: Packages) -> Vec<String> {
    match packages {
        Packages::Host => {
            let mut args = vec!["--workspace".to_string()];
            for name in paths::TARGET_PACKAGES {
                args.extend(["--exclude".to_string(), (*name).to_string()]);
            }
            args
        }
        Packages::Kernel => cargo::package_args(paths::TARGET_PACKAGES),
    }
}
