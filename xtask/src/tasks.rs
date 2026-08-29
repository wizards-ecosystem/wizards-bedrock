//! The check, format, and housekeeping tasks.
//!
//! Each Cargo-driven task runs over every graph in `crate::gate::REGISTRY`,
//! so standalone commands and the complete local gate cannot drift apart.

use std::time::Duration;

use anyhow::{Context, Result};

use crate::cargo::{self, Profile};
use crate::cli::{CleanScope, FmtMode};
use crate::paths::{self, Paths};
use crate::{gate, image, qemu, vendor};

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
    let image = image::build(paths, profile, image::Kernel::SelfTest)?;
    qemu::run_selftest(paths, &image, selftest_timeout(profile))
}

/// Boot a self-test kernel while preserving a valid failed or panic verdict.
///
/// # Errors
///
/// If the kernel cannot be built, QEMU cannot run, or its serial and exit
/// evidence do not form one valid outcome.
pub fn observe_test_boot(paths: &Paths, profile: Profile) -> Result<qemu::SelftestOutcome> {
    let image = image::build(paths, profile, image::Kernel::SelfTest)?;
    qemu::observe_selftest(paths, &image, selftest_timeout(profile))
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
    let image = image::build(paths, profile, image::Kernel::Shipped)?;
    qemu::run_boot_check(paths, &image, BOOT_SETTLE)
}

/// Boot without PCI configuration or x2APIC and require both refusals.
pub fn degraded_check(paths: &Paths, profile: Profile) -> Result<()> {
    let image = image::build_degraded(paths, profile)?;
    qemu::run_degraded_check(paths, &image, BOOT_SETTLE)
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
        if vm_dir.exists() {
            std::fs::remove_dir_all(&vm_dir)
                .with_context(|| format!("removing {}", vm_dir.display()))?;
            println!("[xtask] removed {}", vm_dir.display());
        }
    }
    Ok(())
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

    /// The hole this table closes: the gate used to lint the kernel only with
    /// the self-test feature on, so the configuration that ships was never
    /// linted at all, and two `dead_code` warnings rode a release image out.
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
}
