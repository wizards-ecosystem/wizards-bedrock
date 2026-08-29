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
    vmware            build an image and generate vm/ for VMware Workstation

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
    verify-vendor     check vendored binaries against their CHECKSUMS files
    ci                run the registered local gate shown below

QUALIFICATION TASKS:
    evidence          run CI, reproduce the release twice and write its evidence
    verify-evidence   verify the evidence bundle without network access

";

const HELP_AFTER_REGISTRY: &str = "\
NOT IN CI (each needs a tool this project cannot vendor into a Rust build,
and a gate step that fails for want of one teaches people to skip the gate):
    model-check       run TLC over models/Ring.tla (needs java)
    vmware            as above (needs VMware Workstation to boot the result)
    mutate            break the kernel on purpose, once per known defect, and
                      require a check to go red (one QEMU boot each)

HOUSEKEEPING:
    clean             remove build output

OPTIONS:
    --release         build optimised (default)
    --debug           build unoptimised
    --check           fmt only: fail instead of rewriting
    --all             clean only: also remove the generated vm/ directory
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
    Vmware(Profile),
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
    /// Check the queue protocol's TLA+ model.
    ModelCheck,
    /// Break the kernel on purpose and require the checks to notice.
    Mutate,
    /// Everything CI runs.
    Ci,
    /// Produce a reproducible release evidence bundle.
    Evidence,
    /// Verify a release evidence bundle without network access.
    VerifyEvidence,
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
/// An empty argument list is an error, not a default: the previous behaviour
/// booted a virtual machine whenever a task name was mistyped.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Task> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
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
    let mut passthrough = Vec::new();

    let mut rest = args;
    while let Some(arg) = rest.next() {
        let flag = arg.to_string_lossy().into_owned();
        match flag.as_str() {
            "--release" => profile = Profile::Release,
            "--debug" => profile = Profile::Dev,
            "--check" => check = true,
            "--all" => all = true,
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
        "vmware" => Ok(Task::Vmware(profile)),
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
        "verify-vendor" => Ok(Task::VerifyVendor),
        "model-check" => Ok(Task::ModelCheck),
        "mutate" => Ok(Task::Mutate),
        "ci" => Ok(Task::Ci),
        "evidence" => Ok(Task::Evidence),
        "verify-evidence" => Ok(Task::VerifyEvidence),
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
            parse_args(&["fmt", "--check"]).unwrap(),
            Task::Fmt(FmtMode::Check)
        );
        assert_eq!(parse_args(&["fmt"]).unwrap(), Task::Fmt(FmtMode::Write));
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
        // Read out of the parser rather than restated beside it. A list here
        // would only be a second copy of the match below, and a second copy is
        // green on the day the two disagree — which is the whole failure this
        // test exists to prevent.
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
}
