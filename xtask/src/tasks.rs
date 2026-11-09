//! The check, format, and housekeeping tasks.
//!
//! Each Cargo-driven task runs over every graph in `crate::gate::REGISTRY`,
//! so standalone commands and the complete local gate cannot drift apart.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};

use crate::cargo::{self, Profile};
use crate::cli::{CleanScope, FmtMode};
use crate::paths::{self, Paths};
use crate::{gate, image, qemu, supply_chain, vendor};

/// Whether a subcommand should be pointed at a package's test and bench
/// targets as well as the ones it ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Everything a package builds, test code included. What `check` and
    /// `clippy` want, and the reason the host half sees its own tests.
    AllTargets,
    /// What the subcommand builds by default. `cargo doc` rejects
    /// `--all-targets` outright rather than ignoring it.
    Default,
}

/// Type-check every crate.
pub fn check(paths: &Paths, message_format: Option<&str>) -> Result<()> {
    let extra: Vec<String> = message_format
        .map(|format| vec![format!("--message-format={format}")])
        .unwrap_or_default();
    for_each_pass(paths, "check", "check", Scope::AllTargets, &extra, &[])
}

/// Lint every crate, treating warnings as errors.
///
/// The `-D warnings` goes after `--` on purpose: setting `RUSTFLAGS` instead
/// would replace the target rustflags in `.cargo/config.toml` and silently
/// build a kernel without `code-model=kernel`.
pub fn clippy(paths: &Paths) -> Result<()> {
    for_each_pass(
        paths,
        "clippy",
        "clippy",
        Scope::AllTargets,
        &[],
        &["-D".to_string(), "warnings".to_string()],
    )
}

/// Build the documentation, so the doc-link lints have something to fire on.
pub fn doc(paths: &Paths) -> Result<()> {
    for_each_pass(
        paths,
        "docs",
        "doc",
        Scope::Default,
        &["--no-deps".to_string()],
        &[],
    )
}

/// Format the workspace, or report what is unformatted.
pub fn fmt(paths: &Paths, mode: FmtMode) -> Result<()> {
    let mut args = vec!["fmt".to_string(), "--all".to_string()];
    if mode == FmtMode::Check {
        args.extend(["--".to_string(), "--check".to_string()]);
    }
    cargo::run(paths, "formatting", &args)
}

/// Run the host test suite. The kernel crates have no host tests by
/// construction — see `docs/005-code-conventions.md`.
pub fn test(paths: &Paths) -> Result<()> {
    let mut args = vec!["test".to_string()];
    args.extend(cargo::package_args(paths::HOST_PACKAGES));
    cargo::run(paths, "tests", &args)
}

/// Boot a self-test kernel under QEMU and require its verdict.
///
/// This is the only automated check that exercises the kernel on a machine
/// rather than as arithmetic on the host.
pub fn test_boot(paths: &Paths, profile: Profile) -> Result<()> {
    let image = image::build_for_run(paths, profile, image::Kernel::SelfTest)?;
    qemu::run_selftest(paths, image.path(), selftest_timeout(profile))
}

/// Boot a self-test kernel while preserving a valid failed or panic verdict.
///
/// # Errors
///
/// If the kernel cannot be built, QEMU cannot run, or its serial and exit
/// evidence do not form one valid outcome.
pub fn observe_test_boot(
    paths: &Paths,
    profile: Profile,
    timeout: Duration,
) -> Result<qemu::SelftestOutcome> {
    let image = image::build_for_run(paths, profile, image::Kernel::SelfTest)?;
    qemu::observe_selftest(paths, image.path(), timeout)
}

/// How long a self-test boot may take, by profile.
///
/// A debug kernel runs several times slower under emulation — a console
/// scroll alone costs the better part of a second — so holding it to the
/// release budget would report load as a hang.
const fn selftest_timeout(profile: Profile) -> Duration {
    match profile {
        Profile::Release => SELFTEST_TIMEOUT,
        Profile::Dev => Duration::from_secs(150),
    }
}

/// Generous enough for a cold QEMU start on a loaded machine; short enough that
/// a hung kernel is reported rather than waited on.
const SELFTEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Machine mutations can trigger several bounded red paths before their exact
/// witness. Keep enough room for those paths and the interactive rounds while
/// retaining exact shorter deadlines for mutations whose property is timeout.
pub(crate) const MUTATION_SELFTEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Boot the shipped kernel and require it to reach its steady state.
///
/// The self-test kernel exits at the debug-exit device the moment it has a
/// verdict; this boots the image that goes on the disk.
pub fn boot_check(paths: &Paths, profile: Profile) -> Result<()> {
    let image = image::build_for_run(paths, profile, image::Kernel::Shipped)?;
    boot_shipped_image(paths, image.path())
}

pub(crate) fn boot_shipped_image(paths: &Paths, image: &std::path::Path) -> Result<()> {
    qemu::run_boot_check(paths, image, BOOT_SETTLE)
}

/// Boot without PCI configuration or x2APIC and require both refusals, then
/// boot the shipped kernel without a remapping unit and require storage to
/// refuse rather than run unisolated (D-357).
pub fn degraded_check(paths: &Paths, profile: Profile) -> Result<()> {
    match observe_degraded_boot(paths, profile)? {
        qemu::DegradedOutcome::Panicked {
            transcript,
            no_x2apic_diagnosed,
        } if transcript.contains(qemu::NO_PCI_HANDOFF_PANIC) && !no_x2apic_diagnosed => {
            println!(
                "[xtask] degraded kernel refuses an unprovable DMA handoff before memory initialization"
            );
        }
        qemu::DegradedOutcome::Panicked { transcript, .. } => anyhow::bail!(
            "the degraded kernel reported the wrong terminal handoff refusal: {transcript}"
        ),
        qemu::DegradedOutcome::Settled => anyhow::bail!(
            "the degraded kernel settled without reporting {:?}",
            qemu::NO_PCI_HANDOFF_PANIC
        ),
    }
    match observe_no_x2apic_boot(paths, profile)? {
        qemu::DegradedOutcome::Settled => {
            println!("[xtask] optional CPU protection fallback remains diagnostic-only");
        }
        qemu::DegradedOutcome::Panicked { transcript, .. } => {
            anyhow::bail!("the no-x2APIC kernel panicked during the settle window: {transcript}")
        }
    }
    match observe_unisolated_boot(paths, profile)? {
        qemu::DegradedOutcome::Settled => {
            println!("[xtask] unisolated kernel refuses storage and settles");
            Ok(())
        }
        qemu::DegradedOutcome::Panicked { transcript, .. } => {
            anyhow::bail!("the unisolated kernel panicked during the settle window: {transcript}")
        }
    }
}

/// Boot a degraded kernel while preserving a valid terminal panic.
pub(crate) fn observe_degraded_boot(
    paths: &Paths,
    profile: Profile,
) -> Result<qemu::DegradedOutcome> {
    let image = image::build_degraded_for_run(paths, profile)?;
    qemu::observe_degraded(paths, image.path(), BOOT_SETTLE)
}

/// Boot the shipped kernel on a machine with no remapping unit.
pub(crate) fn observe_unisolated_boot(
    paths: &Paths,
    profile: Profile,
) -> Result<qemu::DegradedOutcome> {
    let image = image::build_for_run(paths, profile, image::Kernel::Shipped)?;
    qemu::observe_unisolated(paths, image.path(), BOOT_SETTLE)
}

/// Boot the shipped kernel without x2APIC but with the DMA handoff intact.
pub(crate) fn observe_no_x2apic_boot(
    paths: &Paths,
    profile: Profile,
) -> Result<qemu::DegradedOutcome> {
    let image = image::build_for_run(paths, profile, image::Kernel::Shipped)?;
    qemu::observe_no_x2apic(paths, image.path(), BOOT_SETTLE)
}

/// Seconds of running after the kernel says it has settled, so one that dies
/// on its first interrupt is still caught.
const BOOT_SETTLE: Duration = Duration::from_secs(5);

/// Everything the gate runs, in the order that fails fastest.
pub fn ci(paths: &Paths) -> Result<()> {
    for &step in gate::REGISTRY.verified_steps(paths.root())? {
        run_ci_step(paths, step)?;
    }
    println!("[xtask] ci passed");
    Ok(())
}

fn run_ci_step(paths: &Paths, step: gate::Step) -> Result<()> {
    match step {
        gate::Step::Format => fmt(paths, FmtMode::Check),
        gate::Step::Clippy => clippy(paths),
        gate::Step::ApiAudit => crate::api::check(paths),
        gate::Step::Documentation => doc(paths),
        gate::Step::HostTests => test(paths),
        gate::Step::SupplyChain => supply_chain::verify(paths),
        gate::Step::Vendor => vendor::verify(paths),
        gate::Step::SelfTest(profile) => test_boot(paths, profile),
        gate::Step::ShippedBoot => boot_check(paths, Profile::Release),
        gate::Step::DegradedBoot => degraded_check(paths, Profile::Release),
    }
}

/// Remove build output.
pub fn clean(paths: &Paths, scope: CleanScope) -> Result<()> {
    let status = cargo::run_status(paths, "clean", ["clean"])?;
    let retained_runner = if status.success() {
        finish_cargo_clean(true, paths.target_dir(), None)?
    } else {
        let current = std::env::current_exe().context("resolving the running clean executable")?;
        finish_cargo_clean(false, paths.target_dir(), Some(&current))
            .with_context(|| format!("clean failed ({status})"))?
    };
    if retained_runner {
        println!(
            "[xtask] Cargo retained only the running clean executable; every other target artifact was removed"
        );
    }
    if scope == CleanScope::Everything {
        let vm_dir = paths.vm_dir();
        if remove_vm_directory(paths.root(), &vm_dir)? {
            println!("[xtask] emptied {}", vm_dir.display());
        }
    }
    Ok(())
}

fn finish_cargo_clean(
    succeeded: bool,
    target: &Path,
    current_executable: Option<&Path>,
) -> Result<bool> {
    if succeeded {
        return Ok(false);
    }
    let current_executable =
        current_executable.context("failed clean has no running executable")?;
    let target =
        crate::mutate::workspace::canonical_existing_real_directory(target, "Cargo clean target")?;
    let profile = target.join("xtask");
    let executable = profile.join(format!("xtask{}", std::env::consts::EXE_SUFFIX));

    let mut target_entries = std::fs::read_dir(&target)
        .with_context(|| format!("listing Cargo clean target {}", target.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    target_entries.sort_by_key(std::fs::DirEntry::file_name);
    ensure!(
        target_entries.len() == 1,
        "failed Cargo clean left an unexpected target-root entry"
    );
    ensure!(
        target_entries[0].path() == profile,
        "failed Cargo clean left an unexpected target-root name"
    );
    let profile_metadata = std::fs::symlink_metadata(&profile)
        .context("inspecting retained xtask profile directory")?;
    ensure!(
        crate::mutate::workspace::metadata_is_real_directory(&profile_metadata)
            && std::fs::canonicalize(&profile)
                .context("canonicalizing retained xtask profile directory")?
                == profile,
        "failed Cargo clean retained a redirected xtask profile directory"
    );

    let mut profile_entries = std::fs::read_dir(&profile)
        .context("listing retained xtask profile directory")?
        .collect::<std::io::Result<Vec<_>>>()?;
    profile_entries.sort_by_key(std::fs::DirEntry::file_name);
    ensure!(
        profile_entries.len() == 1,
        "failed Cargo clean left an unexpected xtask-profile entry"
    );
    ensure!(
        profile_entries[0].path() == executable,
        "failed Cargo clean left an unexpected xtask-profile name"
    );
    let _running =
        crate::mutate::workspace::open_real_file(&executable, "retained running clean executable")?;
    let current_executable = std::fs::canonicalize(current_executable)
        .context("canonicalizing the running clean executable")?;
    ensure!(
        std::fs::canonicalize(&executable)
            .context("canonicalizing the retained running clean executable")?
            == current_executable,
        "failed Cargo clean retained an executable other than its running image"
    );
    Ok(true)
}

fn remove_vm_directory(root: &std::path::Path, directory: &std::path::Path) -> Result<bool> {
    remove_vm_directory_with(root, directory, || {})
}

fn remove_vm_directory_with(
    root: &std::path::Path,
    directory: &std::path::Path,
    before_remove: impl FnOnce(),
) -> Result<bool> {
    ensure!(
        directory == root.join("vm"),
        "VMware cleanup target is not the exact allowlisted repository child"
    );
    let repository =
        crate::mutate::workspace::open_real_directory(root, "VMware cleanup repository root")?;
    let mut options = fs_at::OpenOptions::default();
    options.read(true).follow(false);
    let mut directory_handle = match options.open_dir_at(&repository, "vm") {
        Ok(directory_handle) => directory_handle,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("opening VMware cleanup directory {}", directory.display())
            });
        }
    };
    before_remove();
    let mut remaining = MAX_VM_CLEANUP_ENTRIES;
    empty_open_directory(&mut directory_handle, directory, 0, &mut remaining)?;
    Ok(true)
}

const MAX_VM_CLEANUP_ENTRIES: usize = 100_000;
const MAX_VM_CLEANUP_DEPTH: usize = 256;

fn empty_open_directory(
    directory: &mut std::fs::File,
    display_path: &std::path::Path,
    depth: usize,
    remaining: &mut usize,
) -> Result<()> {
    ensure!(
        depth <= MAX_VM_CLEANUP_DEPTH,
        "VMware cleanup exceeded its {MAX_VM_CLEANUP_DEPTH}-directory depth limit below {}",
        display_path.display()
    );
    let mut options = fs_at::OpenOptions::default();
    options.read(true).follow(false);
    while let Some(name) = next_directory_entry(directory, display_path)? {
        ensure!(
            *remaining != 0,
            "VMware cleanup exceeded its {MAX_VM_CLEANUP_ENTRIES}-entry limit below {}",
            display_path.display()
        );
        *remaining -= 1;
        let child_path = display_path.join(&name);
        match options.open_dir_at(directory, &name) {
            Ok(mut child) => {
                let metadata = child
                    .metadata()
                    .with_context(|| format!("inspecting opened {}", child_path.display()))?;
                if crate::mutate::workspace::metadata_is_real_directory(&metadata) {
                    empty_open_directory(&mut child, &child_path, depth + 1, remaining)?;
                    drop(child);
                    match options.rmdir_at(directory, &name) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(error)
                                .with_context(|| format!("removing {}", child_path.display()));
                        }
                    }
                } else {
                    drop(child);
                    unlink_directory_entry(&options, directory, &name, &child_path, None)?;
                }
            }
            Err(open_error) => {
                unlink_directory_entry(&options, directory, &name, &child_path, Some(&open_error))?
            }
        }
    }
    Ok(())
}

fn unlink_directory_entry(
    options: &fs_at::OpenOptions,
    directory: &std::fs::File,
    name: &std::ffi::OsStr,
    child_path: &std::path::Path,
    open_error: Option<&std::io::Error>,
) -> Result<()> {
    match options.unlink_at(directory, name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(unlink_error) => Err(unlink_error).with_context(|| match open_error {
            Some(error) => format!(
                "removing non-directory {} after directory open failed: {error}",
                child_path.display()
            ),
            None => format!("removing non-directory {}", child_path.display()),
        }),
    }
}

fn next_directory_entry(
    directory: &mut std::fs::File,
    display_path: &std::path::Path,
) -> Result<Option<std::ffi::OsString>> {
    let entries = fs_at::read_dir(directory)
        .with_context(|| format!("reading {}", display_path.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", display_path.display()))?;
        if entry.name() != "." && entry.name() != ".." {
            return Ok(Some(entry.name().to_os_string()));
        }
    }
    Ok(None)
}

/// Run `subcommand` over every registered Cargo graph.
fn for_each_pass(
    paths: &Paths,
    description: &str,
    subcommand: &str,
    scope: Scope,
    extra: &[String],
    after_dashes: &[String],
) -> Result<()> {
    for pass in gate::REGISTRY.cargo_passes() {
        let args = pass_args(subcommand, *pass, scope, extra, after_dashes);
        let label = pass.label();
        cargo::run(paths, &format!("{description} ({label})"), &args)?;
    }
    Ok(())
}

/// The command line for one pass.
///
/// `--all-targets` goes to the host half only, and only for a subcommand that
/// takes it. The kernel crates have no test or bench targets and cannot have
/// any, but the host crates do — and without it neither `check` nor `clippy`
/// ever sees their test code. A `useless_vec` in `sha256.rs`'s tests sat there
/// unreported until someone ran clippy by hand.
fn pass_args(
    subcommand: &str,
    pass: gate::CargoPass,
    scope: Scope,
    extra: &[String],
    after_dashes: &[String],
) -> Vec<String> {
    let mut args = vec![subcommand.to_string()];
    args.extend(cargo::package_args(pass.packages));
    match pass.target {
        Some(target) => args.extend(["--target".to_string(), target.to_string()]),
        None if scope == Scope::AllTargets => args.push("--all-targets".to_string()),
        None => {}
    }
    if let Some(features) = pass.features {
        args.extend(["--features".to_string(), features.to_string()]);
    }
    args.extend_from_slice(extra);
    if !after_dashes.is_empty() {
        args.push("--".to_string());
        args.extend_from_slice(after_dashes);
    }
    args
}

#[cfg(test)]
mod tests;
