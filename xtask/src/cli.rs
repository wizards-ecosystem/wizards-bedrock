//! Argument parsing.
//!
//! Hand-rolled rather than `clap`: a dozen tasks and six flags are a `match`
//! that is itself unit-testable, and this runs on every `cargo xtask`.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::cargo::Profile;

const HELP_BEFORE_REGISTRY: &str = "\
cargo xtask: build orchestrator for The Wizard's Bedrock

USAGE:
    cargo xtask <TASK> [OPTIONS]

BUILD TASKS:
    build             compile the kernel
    image             build a bootable UEFI disk image
    run               build an image and boot it in QEMU
    vmware            build an image and generate a VMware Workstation VM

CHECK TASKS:
    api-audit         prove every public foundation API has a resolved caller
    check             cargo check, host and kernel targets
    clippy            cargo clippy, host and kernel targets
    doc               build the docs, failing on a broken doc link
    fmt               format the workspace
    test              run the host test suite
    test-boot         boot a self-test kernel in QEMU and require its verdict
    boot-check        boot the shipped kernel and require it to settle
    degraded-check    boot without PCI/x2apic, then without an IOMMU; require every refusal
    supply-chain      verify lock, licenses, artifacts and the SPDX SBOM
    verify-vendor     check vendored binaries against their CHECKSUMS files
    ci                run the registered local gate shown below

QUALIFICATION TASKS:
    evidence          run CI, reproduce the release twice and write its evidence
    verify-evidence   verify the evidence bundle without network access
    qualify-advisory  fetch RustSec data and qualify the exact Cargo.lock
    qualify-mutants-grade  run the independent xtask cargo-mutants grade
    qualify-security-deep  run, begin or verify the managed Codex Security deep scan
    qualify-vmware    validate and preserve a fresh VMware serial transcript
    baseline          repeat debug/release foundation measurements in QEMU
    debug             open the debug kernel at kmain in native GDB

";

const HELP_AFTER_REGISTRY: &str = "\
NOT IN CI (exceptional dependency, interaction, or runtime):
    debug --check     as above (needs the pinned native GDB)
    baseline          as above (six descriptive QEMU boots)
    model-check       run TLC over the ring models (needs java)
    vmware            as above (needs VMware Workstation to boot the result)
    qualify-vmware    as above (validates a fresh human-boot transcript)
    mutate            break the kernel on purpose, once per known defect, and
                      require a check to go red (one QEMU boot each)
    qualify-advisory  as above (needs cargo-audit and fresh RustSec data)
    qualify-mutants-grade  as above (a complete run can take several days)
    qualify-security-deep  as above (owned run can qualify; imported verification is diagnostic)

HOUSEKEEPING:
    update-sbom       regenerate the SPDX SBOM from the reviewed inventory
    clean             remove build output (the running xtask image may remain)

OPTIONS:
    --release         build optimised (default)
    --debug           build unoptimised
                      build/image/run/vmware/test-boot/boot-check/degraded-check only
    --check           fmt: do not rewrite; debug: run the bounded setup proof
    --all             clean only: also remove the generated vm/ directory
    --shipped         vmware tasks: select the image users boot, not self-test
    --run             cargo-mutants or Codex Security: run the owned campaign
    --begin           Codex Security: write a source-bound scan request
    --verify=<DIR>    Codex Security: verify unsigned artifacts without qualifying them
    --message-format=<FMT>
                      check only: forwarded to cargo
    -h, --help        print this help
    -- <ARGS>...      run only: pass ARGS through to QEMU
";

/// Usage text, with CI composition rendered from the gate registry.
pub fn help() -> String {
    format!(
        "{HELP_BEFORE_REGISTRY}{}\n{HELP_AFTER_REGISTRY}",
        crate::gate::help_section()
    )
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// Print [`help`].
    Help,
    /// Compile the kernel.
    Build(Profile),
    /// Build a bootable image.
    Image(Profile),
    /// Build an image and boot it in QEMU.
    Run {
        /// Profile to build.
        profile: Profile,
        /// Arguments forwarded to QEMU after `--`.
        qemu_args: Vec<OsString>,
    },
    /// Build an image and generate a VMware VM directory.
    Vmware {
        /// Kernel build profile.
        profile: Profile,
        /// Which image the generated VM boots.
        kernel: crate::image::Kernel,
    },
    /// Type-check every crate.
    Check {
        /// `--message-format` value forwarded to cargo, for editors.
        message_format: Option<String>,
    },
    /// Prove every public foundation API has a compiler-resolved caller.
    ApiAudit,
    /// Lint every crate.
    Clippy,
    /// Build the documentation, so the doc-link lints fire.
    Doc,
    /// Format the workspace.
    Fmt(FmtMode),
    /// Run the host tests.
    Test,
    /// Boot a self-test kernel under QEMU and require its verdict.
    TestBoot(Profile),
    /// Boot the shipped kernel and require it to settle without panicking.
    BootCheck(Profile),
    /// Boot without PCI configuration or x2APIC and require both refusals,
    /// then without a remapping unit and require storage to refuse.
    DegradedCheck(Profile),
    /// Check vendored binaries against their recorded digests.
    VerifyVendor,
    /// Verify the locked supply chain, licenses and generated SPDX document.
    SupplyChain,
    /// Regenerate the deterministic SPDX document.
    UpdateSbom,
    /// Check the queue protocol's TLA+ model.
    ModelCheck,
    /// Repeat the fixed-QEMU foundation performance and resource baseline.
    Baseline,
    /// Open a private paused machine in the supported native debugger.
    Debug(DebugMode),
    /// Break the kernel on purpose and require the checks to notice.
    Mutate,
    /// Everything CI runs.
    Ci,
    /// Produce a reproducible release evidence bundle.
    Evidence,
    /// Verify a release evidence bundle without network access.
    VerifyEvidence,
    /// Qualify the locked dependency graph against fresh RustSec data.
    QualifyAdvisory,
    /// Run the independent cargo-mutants grade under owned execution.
    QualifyMutantsGrade(MutantsGradeAction),
    /// Run, begin or diagnostically verify the managed Codex Security deep scan.
    QualifySecurityDeep(SecurityDeepAction),
    /// Validate and preserve one executed VMware machine.
    QualifyVmware {
        /// Which generated machine supplied the transcript.
        kernel: crate::image::Kernel,
    },
    /// Remove build output.
    Clean(CleanScope),
}

/// Which part of the external cargo-mutants qualification to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutantsGradeAction {
    /// Run and record the complete campaign under `xtask` process ownership.
    Run,
}

/// Which part of the external Codex Security qualification to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityDeepAction {
    /// Run and record the complete scan under `xtask` process ownership.
    Run,
    /// Invalidate an older pass and write a source-bound scan request.
    Begin,
    /// Validate unsigned canonical artifacts without creating a pass record.
    Verify(PathBuf),
}

/// Whether `fmt` rewrites files or just reports on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FmtMode {
    /// Reformat the workspace in place.
    #[default]
    Write,
    /// Leave files alone and fail if any is unformatted.
    Check,
}

/// Whether the debugger stays interactive or proves the setup and exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebugMode {
    /// Leave GDB at the kernel entry until the user quits it.
    #[default]
    Interactive,
    /// Hit the entry, require a symbolized backtrace and exit within a deadline.
    Check,
}

/// How much `clean` removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CleanScope {
    /// Cargo's build output only.
    #[default]
    Build,
    /// Build output and the generated contents below `vm/`, VMware state included.
    Everything,
}

/// Parse the arguments after the task separator.
///
/// Empty input is refused so a mistyped task cannot default to booting a VM.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Task> {
    let mut args = args.into_iter();
    let Some(first) = core::iter::Iterator::next(&mut args) else {
        bail!("no task given");
    };

    let name = first.to_string_lossy().into_owned();
    if name == "-h" || name == "--help" || name == "help" {
        return Ok(Task::Help);
    }

    let mut profile = Profile::default();
    let mut profile_option = false;
    let mut message_format = None;
    let mut check = false;
    let mut all = false;
    let mut shipped = false;
    let mut run = false;
    let mut begin = false;
    let mut verify = None;
    let mut passthrough = Vec::new();
    let mut passthrough_option = false;

    let mut rest = args;
    while let Some(arg) = core::iter::Iterator::next(&mut rest) {
        let flag = arg.to_string_lossy().into_owned();
        match flag.as_str() {
            "--release" => {
                profile = Profile::Release;
                profile_option = true;
            }
            "--debug" => {
                profile = Profile::Dev;
                profile_option = true;
            }
            "--check" => check = true,
            "--all" => all = true,
            "--shipped" => shipped = true,
            "--run" => {
                if run {
                    bail!("duplicate `--run`");
                }
                run = true;
            }
            "--begin" => {
                if begin {
                    bail!("duplicate `--begin`");
                }
                begin = true;
            }
            "-h" | "--help" => return Ok(Task::Help),
            "--" => {
                passthrough_option = true;
                passthrough.extend(rest.by_ref());
                break;
            }
            _ => {
                if let Some(value) = flag.strip_prefix("--message-format=") {
                    message_format = Some(value.to_string());
                } else if let Some(value) = flag.strip_prefix("--verify=") {
                    ensure_nonempty_path(value)?;
                    if verify.is_some() {
                        bail!("duplicate `--verify`");
                    }
                    verify = Some(PathBuf::from(value));
                } else {
                    bail!("unknown option `{flag}`");
                }
            }
        }
    }

    if !matches!(
        name.as_str(),
        "qualify-mutants-grade" | "qualify-security-deep"
    ) && (run || begin || verify.is_some())
    {
        bail!("external qualification options do not apply to `{name}`");
    }

    if profile_option
        && !matches!(
            name.as_str(),
            "build" | "image" | "run" | "vmware" | "test-boot" | "boot-check" | "degraded-check"
        )
    {
        bail!("build profile options do not apply to `{name}`");
    }
    if check && !matches!(name.as_str(), "fmt" | "debug") {
        bail!("`--check` does not apply to `{name}`");
    }
    if all && name != "clean" {
        bail!("`--all` does not apply to `{name}`");
    }
    if shipped && !matches!(name.as_str(), "vmware" | "qualify-vmware") {
        bail!("`--shipped` does not apply to `{name}`");
    }
    if message_format.is_some() && name != "check" {
        bail!("`--message-format` does not apply to `{name}`");
    }
    if passthrough_option && name != "run" {
        bail!("`--` passthrough does not apply to `{name}`");
    }

    match name.as_str() {
        "build" => Ok(Task::Build(profile)),
        "image" => Ok(Task::Image(profile)),
        "run" => Ok(Task::Run {
            profile,
            qemu_args: passthrough,
        }),
        "vmware" => Ok(Task::Vmware {
            profile,
            kernel: if shipped {
                crate::image::Kernel::Shipped
            } else {
                crate::image::Kernel::SelfTest
            },
        }),
        "check" => Ok(Task::Check { message_format }),
        "api-audit" => Ok(Task::ApiAudit),
        "clippy" => Ok(Task::Clippy),
        "doc" => Ok(Task::Doc),
        "fmt" => Ok(Task::Fmt(if check {
            FmtMode::Check
        } else {
            FmtMode::Write
        })),
        "test" => Ok(Task::Test),
        "test-boot" => Ok(Task::TestBoot(profile)),
        "boot-check" => Ok(Task::BootCheck(profile)),
        "degraded-check" => Ok(Task::DegradedCheck(profile)),
        "supply-chain" => Ok(Task::SupplyChain),
        "verify-vendor" => Ok(Task::VerifyVendor),
        "update-sbom" => Ok(Task::UpdateSbom),
        "model-check" => Ok(Task::ModelCheck),
        "baseline" => Ok(Task::Baseline),
        "debug" => Ok(Task::Debug(if check {
            DebugMode::Check
        } else {
            DebugMode::Interactive
        })),
        "mutate" => Ok(Task::Mutate),
        "ci" => Ok(Task::Ci),
        "evidence" => Ok(Task::Evidence),
        "verify-evidence" => Ok(Task::VerifyEvidence),
        "qualify-advisory" => Ok(Task::QualifyAdvisory),
        "qualify-mutants-grade" => match (run, begin, verify) {
            (true, false, None) => Ok(Task::QualifyMutantsGrade(MutantsGradeAction::Run)),
            _ => bail!("qualify-mutants-grade requires exactly `--run`"),
        },
        "qualify-security-deep" => match (run, begin, verify) {
            (true, false, None) => Ok(Task::QualifySecurityDeep(SecurityDeepAction::Run)),
            (false, true, None) => Ok(Task::QualifySecurityDeep(SecurityDeepAction::Begin)),
            (false, false, Some(directory)) => Ok(Task::QualifySecurityDeep(
                SecurityDeepAction::Verify(directory),
            )),
            _ => {
                bail!(
                    "qualify-security-deep requires exactly one of `--run`, `--begin` or \
                     `--verify=<DIR>`"
                )
            }
        },
        "qualify-vmware" => Ok(Task::QualifyVmware {
            kernel: if shipped {
                crate::image::Kernel::Shipped
            } else {
                crate::image::Kernel::SelfTest
            },
        }),
        "clean" => Ok(Task::Clean(if all {
            CleanScope::Everything
        } else {
            CleanScope::Build
        })),
        other => bail!("unknown task `{other}`"),
    }
}

fn ensure_nonempty_path(value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("`--verify` requires a directory");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Task> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_is_an_error_not_run() {
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn builds_are_optimised_by_default() {
        assert_eq!(
            parse_args(&["build"]).unwrap(),
            Task::Build(Profile::Release)
        );
        assert_eq!(
            parse_args(&["build", "--debug"]).unwrap(),
            Task::Build(Profile::Dev)
        );
        assert_eq!(
            parse_args(&["image", "--release"]).unwrap(),
            Task::Image(Profile::Release)
        );
    }

    #[test]
    fn help_is_reachable_every_way_a_user_might_ask() {
        for args in [
            vec!["help"],
            vec!["-h"],
            vec!["--help"],
            vec!["build", "--help"],
        ] {
            assert_eq!(parse_args(&args).unwrap(), Task::Help);
        }
    }

    #[test]
    fn run_forwards_arguments_after_a_double_dash() {
        let task = parse_args(&["run", "--debug", "--", "-display", "none"]).unwrap();
        assert_eq!(
            task,
            Task::Run {
                profile: Profile::Dev,
                qemu_args: vec![OsString::from("-display"), OsString::from("none")],
            }
        );
    }

    #[test]
    fn flags_after_a_double_dash_are_not_ours_to_read() {
        let task = parse_args(&["run", "--", "--debug"]).unwrap();
        assert_eq!(
            task,
            Task::Run {
                profile: Profile::Release,
                qemu_args: vec![OsString::from("--debug")],
            }
        );
    }

    #[test]
    fn task_specific_flags_reach_their_task() {
        assert_eq!(
            parse_args(&["qualify-advisory"]).unwrap(),
            Task::QualifyAdvisory
        );
        assert_eq!(
            parse_args(&["fmt", "--check"]).unwrap(),
            Task::Fmt(FmtMode::Check)
        );
        assert_eq!(parse_args(&["fmt"]).unwrap(), Task::Fmt(FmtMode::Write));
        assert_eq!(
            parse_args(&["debug", "--check"]).unwrap(),
            Task::Debug(DebugMode::Check)
        );
        assert_eq!(
            parse_args(&["debug"]).unwrap(),
            Task::Debug(DebugMode::Interactive)
        );
        assert_eq!(
            parse_args(&["vmware"]).unwrap(),
            Task::Vmware {
                profile: Profile::Release,
                kernel: crate::image::Kernel::SelfTest,
            }
        );
        assert_eq!(
            parse_args(&["vmware", "--shipped"]).unwrap(),
            Task::Vmware {
                profile: Profile::Release,
                kernel: crate::image::Kernel::Shipped,
            }
        );
        assert_eq!(
            parse_args(&["qualify-vmware"]).unwrap(),
            Task::QualifyVmware {
                kernel: crate::image::Kernel::SelfTest,
            }
        );
        assert_eq!(
            parse_args(&["qualify-vmware", "--shipped"]).unwrap(),
            Task::QualifyVmware {
                kernel: crate::image::Kernel::Shipped,
            }
        );
        assert_eq!(
            parse_args(&["clean", "--all"]).unwrap(),
            Task::Clean(CleanScope::Everything)
        );
        assert_eq!(
            parse_args(&["clean"]).unwrap(),
            Task::Clean(CleanScope::Build)
        );
        assert_eq!(
            parse_args(&["check", "--message-format=json"]).unwrap(),
            Task::Check {
                message_format: Some("json".to_string())
            }
        );
    }

    #[test]
    fn unknown_tasks_and_options_are_rejected() {
        assert!(parse_args(&["bulid"]).is_err());
        assert!(parse_args(&["build", "--fast"]).is_err());
    }

    fn assert_refused_elsewhere(options: &[&str], consumers: &[&str]) {
        for task in [
            "build",
            "image",
            "run",
            "vmware",
            "check",
            "api-audit",
            "clippy",
            "doc",
            "fmt",
            "test",
            "test-boot",
            "boot-check",
            "degraded-check",
            "supply-chain",
            "verify-vendor",
            "update-sbom",
            "model-check",
            "baseline",
            "debug",
            "mutate",
            "ci",
            "evidence",
            "verify-evidence",
            "qualify-advisory",
            "qualify-mutants-grade",
            "qualify-security-deep",
            "qualify-vmware",
            "clean",
        ] {
            if consumers.contains(&task) {
                continue;
            }
            let mut arguments = vec![task];
            if matches!(task, "qualify-mutants-grade" | "qualify-security-deep") {
                arguments.push("--run");
            }
            assert!(
                parse_args(&arguments).is_ok(),
                "invalid fixture: {arguments:?}"
            );
            arguments.extend_from_slice(options);
            assert!(
                parse_args(&arguments).is_err(),
                "accepted misplaced option: {arguments:?}"
            );
        }
    }

    #[test]
    fn profile_options_only_reach_tasks_with_a_build_profile() {
        for (option, profile) in [("--release", Profile::Release), ("--debug", Profile::Dev)] {
            for (task, expected) in [
                ("build", Task::Build(profile)),
                ("image", Task::Image(profile)),
                (
                    "run",
                    Task::Run {
                        profile,
                        qemu_args: Vec::new(),
                    },
                ),
                (
                    "vmware",
                    Task::Vmware {
                        profile,
                        kernel: crate::image::Kernel::SelfTest,
                    },
                ),
                ("test-boot", Task::TestBoot(profile)),
                ("boot-check", Task::BootCheck(profile)),
                ("degraded-check", Task::DegradedCheck(profile)),
            ] {
                assert_eq!(parse_args(&[task, option]).unwrap(), expected);
            }
            assert_refused_elsewhere(
                &[option],
                &[
                    "build",
                    "image",
                    "run",
                    "vmware",
                    "test-boot",
                    "boot-check",
                    "degraded-check",
                ],
            );
        }
    }

    #[test]
    fn check_options_cannot_dispatch_cleanup_or_another_unrelated_task() {
        assert_eq!(
            parse_args(&["fmt", "--check"]).unwrap(),
            Task::Fmt(FmtMode::Check)
        );
        assert_eq!(
            parse_args(&["debug", "--check"]).unwrap(),
            Task::Debug(DebugMode::Check)
        );
        let error = parse_args(&["clean", "--check"]).unwrap_err().to_string();
        assert_eq!(error, "`--check` does not apply to `clean`");
        assert_refused_elsewhere(&["--check"], &["fmt", "debug"]);
    }

    #[test]
    fn all_options_only_expand_the_clean_task() {
        assert_eq!(
            parse_args(&["clean", "--all"]).unwrap(),
            Task::Clean(CleanScope::Everything)
        );
        assert_refused_elsewhere(&["--all"], &["clean"]);
    }

    #[test]
    fn shipped_options_only_select_vmware_images() {
        assert_eq!(
            parse_args(&["vmware", "--shipped"]).unwrap(),
            Task::Vmware {
                profile: Profile::Release,
                kernel: crate::image::Kernel::Shipped,
            }
        );
        assert_eq!(
            parse_args(&["qualify-vmware", "--shipped"]).unwrap(),
            Task::QualifyVmware {
                kernel: crate::image::Kernel::Shipped,
            }
        );
        assert_refused_elsewhere(&["--shipped"], &["vmware", "qualify-vmware"]);
    }

    #[test]
    fn message_format_options_only_reach_check_even_when_empty() {
        for (option, value) in [("--message-format=json", "json"), ("--message-format=", "")] {
            assert_eq!(
                parse_args(&["check", option]).unwrap(),
                Task::Check {
                    message_format: Some(value.to_string()),
                }
            );
            assert_refused_elsewhere(&[option], &["check"]);
        }
    }

    #[test]
    fn passthrough_is_run_only_even_when_the_separator_has_no_tail() {
        assert_eq!(
            parse_args(&["run", "--"]).unwrap(),
            Task::Run {
                profile: Profile::Release,
                qemu_args: Vec::new(),
            }
        );
        assert_refused_elsewhere(&["--"], &["run"]);
        assert_refused_elsewhere(&["--", "-display", "none"], &["run"]);
    }

    #[test]
    fn help_and_repeated_ordinary_options_keep_their_existing_precedence() {
        for option in [
            "--release",
            "--debug",
            "--check",
            "--all",
            "--shipped",
            "--message-format=json",
        ] {
            assert_eq!(
                parse_args(&["clean", option, "--help"]).unwrap(),
                Task::Help
            );
        }
        assert_eq!(
            parse_args(&["build", "--debug", "--release"]).unwrap(),
            Task::Build(Profile::Release)
        );
        assert_eq!(
            parse_args(&["build", "--release", "--debug"]).unwrap(),
            Task::Build(Profile::Dev)
        );
        assert_eq!(
            parse_args(&["fmt", "--check", "--check"]).unwrap(),
            Task::Fmt(FmtMode::Check)
        );
        assert_eq!(
            parse_args(&["clean", "--all", "--all"]).unwrap(),
            Task::Clean(CleanScope::Everything)
        );
        assert_eq!(
            parse_args(&["vmware", "--shipped", "--shipped"]).unwrap(),
            Task::Vmware {
                profile: Profile::Release,
                kernel: crate::image::Kernel::Shipped,
            }
        );
        assert_eq!(
            parse_args(&["check", "--message-format=short", "--message-format=json"]).unwrap(),
            Task::Check {
                message_format: Some("json".to_string()),
            }
        );
        assert_eq!(
            parse_args(&["run", "--", "--help"]).unwrap(),
            Task::Run {
                profile: Profile::Release,
                qemu_args: vec![OsString::from("--help")],
            }
        );
    }

    /// A task the parser accepts but the help text never mentions is a task
    /// nobody will run; one the help offers but the parser rejects is worse.
    #[test]
    fn every_task_the_parser_accepts_is_named_in_the_help() {
        // Derive this from the parser so the test cannot drift through a
        // second handwritten task list.
        let names: Vec<&str> = include_str!("cli.rs")
            .lines()
            .filter(|line| {
                line.contains("=> Ok(Task::") || line.contains("=> match (run, begin, verify)")
            })
            .filter_map(|line| line.trim().strip_prefix('"'))
            .filter_map(|rest| rest.split_once('"').map(|(name, _)| name))
            .collect();
        assert!(
            names.len() > 10,
            "the parser was read as {} tasks, so the reading is wrong",
            names.len()
        );

        let help = help();
        for name in names {
            let arguments = if name == "qualify-mutants-grade" {
                vec![name, "--run"]
            } else if name == "qualify-security-deep" {
                vec![name, "--begin"]
            } else {
                vec![name]
            };
            assert!(parse_args(&arguments).is_ok(), "{name} does not parse");
            assert!(
                help.contains(&format!("\n    {name} ")),
                "{name} is missing from the help text"
            );
        }
    }

    #[test]
    fn the_doc_task_parses() {
        assert_eq!(parse_args(&["doc"]).unwrap(), Task::Doc);
    }

    #[test]
    fn external_qualification_actions_are_typed_and_task_local() {
        assert_eq!(
            parse_args(&["qualify-mutants-grade", "--run"]).unwrap(),
            Task::QualifyMutantsGrade(MutantsGradeAction::Run)
        );
        assert_eq!(
            parse_args(&["qualify-security-deep", "--verify=scan"]).unwrap(),
            Task::QualifySecurityDeep(SecurityDeepAction::Verify(PathBuf::from("scan")))
        );
        assert_eq!(
            parse_args(&["qualify-security-deep", "--run"]).unwrap(),
            Task::QualifySecurityDeep(SecurityDeepAction::Run)
        );
        for task in ["qualify-mutants-grade", "qualify-security-deep"] {
            assert!(parse_args(&[task]).is_err(), "{task} ran with no action");
        }
        assert!(parse_args(&["qualify-mutants-grade", "--begin"]).is_err());
        assert!(parse_args(&["qualify-mutants-grade", "--verify=scan"]).is_err());
        assert!(parse_args(&["qualify-mutants-grade", "--run", "--begin"]).is_err());
        assert!(parse_args(&["qualify-mutants-grade", "--run", "--verify=scan"]).is_err());
        assert!(parse_args(&["qualify-security-deep", "--run", "--begin"]).is_err());
        assert!(parse_args(&["qualify-security-deep", "--run", "--verify=scan"]).is_err());
        assert!(parse_args(&["qualify-security-deep", "--begin", "--verify=scan"]).is_err());
        assert!(parse_args(&["qualify-security-deep", "--run", "--run"]).is_err());
        assert!(parse_args(&["qualify-security-deep", "--verify=one", "--verify=two"]).is_err());
        assert!(parse_args(&["build", "--begin"]).is_err());
        assert!(parse_args(&["build", "--verify=scan"]).is_err());
        assert!(parse_args(&["build", "--run"]).is_err());
    }

    #[test]
    fn unsigned_external_result_directories_have_no_seal_action() {
        for task in ["qualify-mutants-grade", "qualify-security-deep"] {
            let error = parse_args(&[task, "--seal=unsigned-bundle"])
                .unwrap_err()
                .to_string();
            assert!(error.contains("unknown option `--seal=unsigned-bundle`"));
        }
    }

    #[test]
    fn every_exceptional_task_is_named_in_the_not_in_ci_block() {
        let help = help();
        let block = help
            .split_once("NOT IN CI (exceptional dependency, interaction, or runtime):\n")
            .unwrap()
            .1
            .split_once("\nHOUSEKEEPING:")
            .unwrap()
            .0;
        let names: std::collections::BTreeSet<&str> = block
            .lines()
            .filter(|line| line.starts_with("    ") && !line.starts_with("                      "))
            .filter_map(|line| core::iter::Iterator::next(&mut line.split_whitespace()))
            .map(|name| name.split_once(' ').map_or(name, |(task, _)| task))
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from([
                "baseline",
                "debug",
                "model-check",
                "mutate",
                "qualify-advisory",
                "qualify-mutants-grade",
                "qualify-security-deep",
                "qualify-vmware",
                "vmware",
            ])
        );
    }
}
