//! The check, format, and housekeeping tasks.
//!
//! Each Cargo-driven task runs over every graph in `crate::gate::REGISTRY`,
//! so standalone commands and the complete local gate cannot drift apart.

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
///
/// `missing_docs` notices a doc comment that is absent; nothing notices one
/// that points at a name since renamed or made private, because
/// `rustdoc::broken_intra_doc_links` only fires under rustdoc — which no other
/// gate step runs. The lints themselves are declared in `[workspace.lints]`,
/// so an editor reports them too; this is what makes them a gate.
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
pub fn observe_test_boot(paths: &Paths, profile: Profile) -> Result<qemu::SelftestOutcome> {
    let image = image::build_for_run(paths, profile, image::Kernel::SelfTest)?;
    qemu::observe_selftest(paths, image.path(), selftest_timeout(profile))
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
    cargo::run(paths, "clean", ["clean"])?;
    if scope == CleanScope::Everything {
        let vm_dir = paths.vm_dir();
        if remove_vm_directory(paths.root(), &vm_dir)? {
            println!("[xtask] emptied {}", vm_dir.display());
        }
    }
    Ok(())
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
        crate::mutation_workspace::open_real_directory(root, "VMware cleanup repository root")?;
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
                if crate::mutation_workspace::metadata_is_real_directory(&metadata) {
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
mod tests {
    use super::*;

    static NEXT_CLEAN_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    #[cfg(unix)]
    fn create_directory_symlink(target: &std::path::Path, link: &std::path::Path) {
        std::os::unix::fs::symlink(target, link).expect("creating cleanup directory symlink");
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &std::path::Path, link: &std::path::Path) {
        std::os::windows::fs::symlink_dir(target, link)
            .expect("creating cleanup directory symlink");
    }

    #[cfg(unix)]
    fn remove_directory_symlink(path: &std::path::Path) {
        std::fs::remove_file(path).expect("removing cleanup directory symlink");
    }

    #[cfg(windows)]
    fn remove_directory_symlink(path: &std::path::Path) {
        std::fs::remove_dir(path).expect("removing cleanup directory symlink");
    }

    fn clean_fixture(label: &str) -> std::path::PathBuf {
        let nonce = NEXT_CLEAN_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let fixture = std::env::temp_dir().join(format!(
            "wizard-clean-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&fixture).expect("cleanup fixture");
        fixture
    }

    const PIXEL: &[&str] = &["pixel"];

    fn pass(target: Option<&'static str>, features: Option<&'static str>) -> gate::CargoPass {
        gate::CargoPass {
            name: "fixture",
            packages: PIXEL,
            target,
            features,
        }
    }

    fn args(target: Option<&'static str>) -> Vec<String> {
        pass_args(
            "clippy",
            pass(target, None),
            Scope::AllTargets,
            &[],
            &["-D".to_string()],
        )
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
    fn a_pass_with_features_names_them_and_one_without_names_none() {
        let with = pass_args(
            "clippy",
            pass(
                Some(paths::KERNEL_TARGET),
                Some(paths::SELFTEST_FEATURE_SPEC),
            ),
            Scope::AllTargets,
            &[],
            &[],
        );
        let position = with.iter().position(|arg| arg == "--features");
        assert_eq!(
            position.map(|index| with[index + 1].as_str()),
            Some(paths::SELFTEST_FEATURE_SPEC)
        );

        assert!(!args(Some(paths::KERNEL_TARGET)).contains(&"--features".to_string()));
    }

    #[test]
    fn the_host_half_has_no_kernel_features() {
        assert!(!args(None).contains(&"--features".to_string()));
    }

    #[test]
    fn nothing_follows_a_bare_double_dash() {
        let args = pass_args("check", pass(None, None), Scope::AllTargets, &[], &[]);
        assert!(!args.contains(&"--".to_string()));
    }

    /// `cargo doc` rejects `--all-targets` rather than ignoring it, so the
    /// scope has to be part of the call and not inferred from the graph.
    #[test]
    fn a_default_scope_asks_for_no_extra_targets() {
        let args = pass_args("doc", pass(None, None), Scope::Default, &[], &[]);

        assert_eq!(args, ["doc", "--package", "pixel"]);
    }

    /// Both shipped and self-test feature graphs must be linted because one
    /// graph can hide dead code in the other.
    #[test]
    fn the_gate_covers_every_kernel_feature_graph_it_boots() {
        let kernel: Vec<Option<&str>> = gate::REGISTRY
            .cargo_passes()
            .iter()
            .filter(|pass| pass.packages == paths::KERNEL_PACKAGES)
            .map(|pass| pass.features)
            .collect();

        assert_eq!(
            kernel,
            [
                None,
                Some(paths::SELFTEST_FEATURE_SPEC),
                Some(paths::REFUSE_PCI_FEATURE_SPEC)
            ]
        );
    }

    #[test]
    fn the_gate_covers_the_host_packages_exactly_once() {
        let host = gate::REGISTRY
            .cargo_passes()
            .iter()
            .filter(|pass| pass.packages == paths::HOST_PACKAGES)
            .count();

        assert_eq!(host, 1);
    }

    /// Every pass has to be distinguishable in the log, or a failure in one
    /// configuration reads as a failure in the other.
    #[test]
    fn every_pass_is_labelled_distinctly() {
        let labels: Vec<String> = gate::REGISTRY
            .cargo_passes()
            .iter()
            .map(|pass| pass.label())
            .collect();

        for (index, label) in labels.iter().enumerate() {
            assert!(!label.is_empty());
            assert!(
                !labels[index + 1..].contains(label),
                "two passes both called {label}"
            );
        }
    }

    /// Kernel passes are cross-compiled and host passes are not — the property
    /// that stops a kernel crate being checked for the host triple, which is
    /// what produces phantom errors across every kernel file.
    #[test]
    fn only_the_host_pass_builds_for_the_host() {
        for pass in gate::REGISTRY.cargo_passes() {
            let cross = pass.target == Some(paths::KERNEL_TARGET);
            assert_eq!(
                cross,
                pass.packages == paths::KERNEL_PACKAGES,
                "{}",
                pass.label()
            );
        }
    }

    #[test]
    fn recursive_vm_cleanup_is_confined_to_one_real_repository_child() {
        let fixture = clean_fixture("confined");
        let repository = fixture.join("repository");
        std::fs::create_dir_all(repository.join("vm/nested/deeper")).unwrap();
        std::fs::write(repository.join("vm/root-file"), b"generated root state").unwrap();
        std::fs::write(
            repository.join("vm/nested/deeper/generated"),
            b"generated nested state",
        )
        .unwrap();
        let outside = repository.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let sentinel = outside.join("sentinel");
        std::fs::write(&sentinel, b"outside authority").unwrap();

        assert!(remove_vm_directory(&repository, &outside).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        assert!(remove_vm_directory(&repository, &repository.join("vm")).unwrap());
        assert!(repository.join("vm").is_dir());
        assert_eq!(std::fs::read_dir(repository.join("vm")).unwrap().count(), 0);
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn recursive_vm_cleanup_refuses_redirected_paths_without_touching_the_target() {
        let fixture = clean_fixture("redirected");
        let repository = fixture.join("repository");
        let redirected = repository.join("redirected");
        std::fs::create_dir_all(&redirected).unwrap();
        let sentinel = redirected.join("sentinel");
        std::fs::write(&sentinel, b"outside authority").unwrap();
        create_directory_symlink(&redirected, &repository.join("vm"));

        assert!(remove_vm_directory(&repository, &repository.join("vm")).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        remove_directory_symlink(&repository.join("vm"));
        std::fs::create_dir(repository.join("vm")).unwrap();
        create_directory_symlink(&redirected, &repository.join("vm/redirected"));

        assert!(remove_vm_directory(&repository, &repository.join("vm")).unwrap());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        assert_eq!(std::fs::read_dir(repository.join("vm")).unwrap().count(), 0);

        let linked_repository = fixture.join("repository-link");
        create_directory_symlink(&repository, &linked_repository);
        assert!(remove_vm_directory(&linked_repository, &linked_repository.join("vm")).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        remove_directory_symlink(&linked_repository);

        let linked_parent = fixture.join("repository-parent-link");
        create_directory_symlink(&fixture, &linked_parent);
        let repository_below_link = linked_parent.join("repository");
        assert!(
            remove_vm_directory(&repository_below_link, &repository_below_link.join("vm")).is_err()
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        remove_directory_symlink(&linked_parent);
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn recursive_vm_cleanup_stays_bound_when_a_validated_ancestor_is_replaced() {
        let fixture = clean_fixture("ancestor-race");
        let repository = fixture.join("repository");
        let moved_repository = fixture.join("repository-before-swap");
        let outside = fixture.join("outside");
        std::fs::create_dir_all(repository.join("vm/nested")).unwrap();
        std::fs::write(repository.join("vm/nested/generated"), b"generated state").unwrap();
        std::fs::create_dir_all(outside.join("vm")).unwrap();
        let sentinel = outside.join("vm/sentinel");
        std::fs::write(&sentinel, b"outside authority").unwrap();
        let mut swapped = false;

        let result = remove_vm_directory_with(&repository, &repository.join("vm"), || {
            if std::fs::rename(&repository, &moved_repository).is_ok() {
                std::fs::rename(&outside, &repository).unwrap();
                swapped = true;
            }
        });

        if swapped {
            std::fs::rename(&repository, &outside).unwrap();
            std::fs::rename(&moved_repository, &repository).unwrap();
        }
        let sentinel_after = std::fs::read(&sentinel);
        std::fs::remove_dir_all(&fixture).unwrap();

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(sentinel_after.unwrap(), b"outside authority");
    }

    #[test]
    fn recursive_vm_cleanup_has_an_entry_budget_before_deleting() {
        let fixture = clean_fixture("entry-budget");
        let directory = fixture.join("vm");
        std::fs::create_dir(&directory).unwrap();
        let sentinel = directory.join("sentinel");
        std::fs::write(&sentinel, b"generated state").unwrap();
        let mut handle =
            crate::mutation_workspace::open_real_directory(&directory, "cleanup fixture").unwrap();
        let mut remaining = 0;

        let error = empty_open_directory(&mut handle, &directory, 0, &mut remaining)
            .expect_err("an exhausted cleanup budget was accepted");
        assert!(error.to_string().contains("entry limit"));
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"generated state");

        drop(handle);
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn recursive_vm_cleanup_has_a_depth_bound_before_deleting() {
        let fixture = clean_fixture("depth-bound");
        let directory = fixture.join("vm");
        std::fs::create_dir(&directory).unwrap();
        let sentinel = directory.join("sentinel");
        std::fs::write(&sentinel, b"generated state").unwrap();
        let mut handle =
            crate::mutation_workspace::open_real_directory(&directory, "cleanup fixture").unwrap();
        let mut remaining = MAX_VM_CLEANUP_ENTRIES;

        let error = empty_open_directory(
            &mut handle,
            &directory,
            MAX_VM_CLEANUP_DEPTH + 1,
            &mut remaining,
        )
        .expect_err("an over-depth cleanup was accepted");
        assert!(error.to_string().contains("depth limit"));
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"generated state");

        drop(handle);
        std::fs::remove_dir_all(fixture).unwrap();
    }
}
