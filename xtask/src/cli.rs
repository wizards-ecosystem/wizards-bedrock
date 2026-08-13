//! Argument parsing.
//!
//! Hand-rolled rather than `clap`: a dozen tasks and six flags are a `match`
//! that is itself unit-testable, and this runs on every `cargo xtask`.

use std::ffi::OsString;

use anyhow::{Result, bail};

use crate::cargo::Profile;

/// Usage text, printed for `--help` and on any parse error.
pub const HELP: &str = "\
cargo xtask — build orchestrator for The Wizard's OS

USAGE:
    cargo xtask <TASK> [OPTIONS]

BUILD TASKS:
    build             compile the kernel
    image             build a bootable UEFI disk image
    run               build an image and boot it in QEMU
    vmware            build an image and generate vm/ for VMware Workstation

CHECK TASKS:
    check             cargo check, host and kernel targets
    clippy            cargo clippy, host and kernel targets
    fmt               format the workspace
    test              run the host test suite
    test-boot         boot a self-test kernel in QEMU and require its verdict
    verify-vendor     check vendored binaries against boot/limine/CHECKSUMS
    ci                every check above, in the order that fails fastest

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

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// Print [`HELP`].
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
    /// Lint every crate.
    Clippy,
    /// Format the workspace.
    Fmt(FmtMode),
    /// Run the host tests.
    Test,
    /// Boot a self-test kernel under QEMU and require its verdict.
    TestBoot(Profile),
    /// Check vendored binaries against their recorded digests.
    VerifyVendor,
    /// Everything CI runs.
    Ci,
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
        "clippy" => Ok(Task::Clippy),
        "fmt" => Ok(Task::Fmt(if check {
            FmtMode::Check
        } else {
            FmtMode::Write
        })),
        "test" => Ok(Task::Test),
        "test-boot" => Ok(Task::TestBoot(profile)),
        "verify-vendor" => Ok(Task::VerifyVendor),
        "ci" => Ok(Task::Ci),
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
}
