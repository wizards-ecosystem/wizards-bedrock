//! Argument parsing.
//!
//! Hand-rolled rather than `clap`: a dozen tasks and six flags are a `match`
//! that is itself unit-testable, and this runs on every `cargo xtask`.

use std::ffi::OsString;

use anyhow::{Result, bail};

use crate::cargo::Profile;

const HELP_BEFORE_REGISTRY: &str = "\
cargo xtask — build orchestrator for The Wizard's OS

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
    degraded-check    boot without PCI/x2apic and require both refusals
    supply-chain      verify lock, licenses, artifacts and the SPDX SBOM
    verify-vendor     check vendored binaries against their CHECKSUMS files
    ci                run the registered local gate shown below

QUALIFICATION TASKS:
    evidence          run CI, reproduce the release twice and write its evidence
    verify-evidence   verify the evidence bundle without network access
    qualify-advisory  fetch RustSec data and qualify the exact Cargo.lock
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

HOUSEKEEPING:
    update-sbom       regenerate the SPDX SBOM from the reviewed inventory
    clean             remove build output

OPTIONS:
    --release         build optimised (default)
    --debug           build unoptimised
    --check           fmt: do not rewrite; debug: run the bounded setup proof
    --all             clean only: also remove the generated vm/ directory
    --shipped         vmware tasks: select the image users boot, not self-test
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
    /// Boot without PCI configuration or x2APIC and require both refusals.
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
    /// Validate and preserve one executed VMware machine.
    QualifyVmware {
        /// Which generated machine supplied the transcript.
        kernel: crate::image::Kernel,
    },
    /// Remove build output.
    Clean(CleanScope),
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
    /// Build output and the generated `vm/` directory, VMware state included.
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
    let mut message_format = None;
    let mut check = false;
    let mut all = false;
    let mut shipped = false;
    let mut passthrough = Vec::new();

    let mut rest = args;
    while let Some(arg) = core::iter::Iterator::next(&mut rest) {
        let flag = arg.to_string_lossy().into_owned();
        match flag.as_str() {
            "--release" => profile = Profile::Release,
            "--debug" => profile = Profile::Dev,
            "--check" => check = true,
            "--all" => all = true,
            "--shipped" => shipped = true,
            "-h" | "--help" => return Ok(Task::Help),
            "--" => {
                passthrough.extend(rest.by_ref());
                break;
            }
            _ => match flag.strip_prefix("--message-format=") {
                Some(value) => message_format = Some(value.to_string()),
                None => bail!("unknown option `{flag}`"),
            },
        }
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

    /// A task the parser accepts but the help text never mentions is a task
    /// nobody will run; one the help offers but the parser rejects is worse.
    #[test]
    fn every_task_the_parser_accepts_is_named_in_the_help() {
        // Derive this from the parser so the test cannot drift through a
        // second handwritten task list.
        let names: Vec<&str> = include_str!("cli.rs")
            .lines()
            .filter(|line| line.contains("=> Ok(Task::"))
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
            assert!(parse_args(&[name]).is_ok(), "{name} does not parse");
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
                "qualify-vmware",
                "vmware",
            ])
        );
    }
}
