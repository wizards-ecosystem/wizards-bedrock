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

ENVIRONMENT:
    QEMU              path to qemu-system-x86_64; overrides QEMU_DIR and PATH
    QEMU_DIR          directory containing that executable
    GDB               path to the pinned WinLibs gdb used by debug
";

/// Usage text, with CI composition rendered from the gate registry.
pub fn help() -> String {
    format!(
        "{HELP_BEFORE_REGISTRY}{}\n{HELP_AFTER_REGISTRY}",
        crate::gate::help_section()
    )
}

/// What the user asked for.
#[derive(Clone, PartialEq, Eq, Debug)]
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
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MutantsGradeAction {
    /// Run and record the complete campaign under `xtask` process ownership.
    Run,
}

/// Which part of the external Codex Security qualification to perform.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SecurityDeepAction {
    /// Run and record the complete scan under `xtask` process ownership.
    Run,
    /// Invalidate an older pass and write a source-bound scan request.
    Begin,
    /// Validate unsigned canonical artifacts without creating a pass record.
    Verify(PathBuf),
}

/// Whether `fmt` rewrites files or just reports on them.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum FmtMode {
    /// Reformat the workspace in place.
    #[default]
    Write,
    /// Leave files alone and fail if any is unformatted.
    Check,
}

/// Whether the debugger stays interactive or proves the setup and exits.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum DebugMode {
    /// Leave GDB at the kernel entry until the user quits it.
    #[default]
    Interactive,
    /// Hit the entry, require a symbolized backtrace and exit within a deadline.
    Check,
}

/// How much `clean` removes.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum CleanScope {
    /// Cargo's build output only.
    #[default]
    Build,
    /// Build output and the generated contents below `vm/`, VMware state included.
    Everything,
}

/// Every task [`parse`] accepts, in the order of its match.
const TASK_NAMES: &[&str] = &[
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
];

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
                } else if name == "run" {
                    bail!("unknown option `{flag}`; pass QEMU arguments after `--`");
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
        other => match nearest_task(other) {
            Some(task) => bail!("unknown task `{other}`; did you mean `{task}`?"),
            None => bail!("unknown task `{other}`"),
        },
    }
}

fn nearest_task(name: &str) -> Option<&'static str> {
    let limit = (name.chars().count().saturating_sub(1) / 2).min(2);
    TASK_NAMES
        .iter()
        .map(|task| (edit_distance(name, task), *task))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, task)| task)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.iter().enumerate() {
            let substitution = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn ensure_nonempty_path(value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("`--verify` requires a directory");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
