use std::ffi::OsString;

use anyhow::{Result, bail};

use crate::cargo::Profile;
use crate::image::Kernel;

pub const HELP: &str = "\
cargo xtask: build orchestrator for The Wizard's Bedrock

USAGE:
    cargo xtask <TASK> [OPTIONS]

BUILD TASKS:
    build             compile the kernel
    image             build a bootable UEFI disk image
    run               build an image and boot it in QEMU
    vmware            build an image and generate a VMware Workstation VM

CHECK TASKS:
    check             cargo check, host and kernel targets
    clippy            cargo clippy, host and kernel targets
    fmt               format the workspace
    test              run the host test suite
    test-boot         boot a self-test kernel in QEMU and require its verdict
    boot-check        boot the shipped kernel and require it to settle
    degraded-check    boot without x2apic, then without an IOMMU; require every refusal
    supply-chain      verify lock sources, licenses and the inventory
    verify-vendor     check vendored binaries against their CHECKSUMS files
    ci                fmt --check, clippy, test, supply-chain, verify-vendor,
                      test-boot --release, test-boot --debug, boot-check, degraded-check

NOT IN CI:
    model-check       run TLC over the ring models (needs java)
    qualify-vmware    check the serial log of a VMware boot of the generated machine
    debug             open the debug kernel at kmain in GDB
    clean             remove build output

OPTIONS:
    --release         build optimised (default)
    --debug           build unoptimised
                      build/image/run/test-boot/boot-check/degraded-check only
    --check           fmt only: do not rewrite
    --all             clean only: also remove the generated vm/ directory
    --shipped         vmware tasks: select the image users boot, not self-test
    --message-format=<FMT>
                      check only: forwarded to cargo
    -h, --help        print this help
    -- <ARGS>...      run only: pass ARGS through to QEMU

ENVIRONMENT:
    QEMU              path to qemu-system-x86_64; overrides QEMU_DIR and PATH
    QEMU_DIR          directory containing that executable
    QEMU_FIRMWARE_CODE, QEMU_FIRMWARE_VARS
                      edk2 code and variable-store images; override the search
    GDB               gdb used by debug; defaults to gdb on PATH
";

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Task {
    Help,
    Build(Profile),
    Image(Profile),
    Run {
        profile: Profile,
        qemu_args: Vec<OsString>,
    },
    Vmware {
        kernel: Kernel,
    },
    QualifyVmware {
        kernel: Kernel,
    },
    Check {
        message_format: Option<String>,
    },
    Clippy,
    Fmt(FmtMode),
    Test,
    TestBoot(Profile),
    BootCheck(Profile),
    DegradedCheck(Profile),
    VerifyVendor,
    SupplyChain,
    ModelCheck,
    Debug,
    Ci,
    Clean(CleanScope),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FmtMode {
    Write,
    Check,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CleanScope {
    Build,
    Everything,
}

#[derive(Default, Debug)]
struct Options {
    profile: Option<Profile>,
    message_format: Option<String>,
    check: bool,
    all: bool,
    shipped: bool,
    passthrough: Option<Vec<OsString>>,
}

pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Task> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        bail!("no task given");
    };
    let name = first.to_string_lossy().into_owned();
    if matches!(name.as_str(), "-h" | "--help" | "help") {
        return Ok(Task::Help);
    }

    let mut options = Options::default();
    while let Some(argument) = args.next() {
        let flag = argument.to_string_lossy().into_owned();
        match flag.as_str() {
            "--release" => options.profile = Some(Profile::Release),
            "--debug" => options.profile = Some(Profile::Dev),
            "--check" => options.check = true,
            "--all" => options.all = true,
            "--shipped" => options.shipped = true,
            "-h" | "--help" => return Ok(Task::Help),
            "--" => {
                options.passthrough = Some(args.by_ref().collect());
                break;
            }
            _ => match flag.strip_prefix("--message-format=") {
                Some(format) => options.message_format = Some(format.to_string()),
                None => bail!("unknown option `{flag}`"),
            },
        }
    }
    require_options_apply(&name, &options)?;
    task_named(&name, options)
}

fn require_options_apply(name: &str, options: &Options) -> Result<()> {
    let profiled = matches!(
        name,
        "build" | "image" | "run" | "test-boot" | "boot-check" | "degraded-check"
    );
    if options.profile.is_some() && !profiled {
        bail!("build profile options do not apply to `{name}`");
    }
    if options.check && name != "fmt" {
        bail!("`--check` does not apply to `{name}`");
    }
    if options.all && name != "clean" {
        bail!("`--all` does not apply to `{name}`");
    }
    if options.shipped && !matches!(name, "vmware" | "qualify-vmware") {
        bail!("`--shipped` does not apply to `{name}`");
    }
    if options.message_format.is_some() && name != "check" {
        bail!("`--message-format` does not apply to `{name}`");
    }
    if options.passthrough.is_some() && name != "run" {
        bail!("`--` passthrough does not apply to `{name}`");
    }
    Ok(())
}

fn task_named(name: &str, options: Options) -> Result<Task> {
    let profile = options.profile.unwrap_or_default();
    let kernel = if options.shipped {
        Kernel::Shipped
    } else {
        Kernel::SelfTest
    };
    Ok(match name {
        "build" => Task::Build(profile),
        "image" => Task::Image(profile),
        "run" => Task::Run {
            profile,
            qemu_args: options.passthrough.unwrap_or_default(),
        },
        "vmware" => Task::Vmware { kernel },
        "qualify-vmware" => Task::QualifyVmware { kernel },
        "check" => Task::Check {
            message_format: options.message_format,
        },
        "clippy" => Task::Clippy,
        "fmt" if options.check => Task::Fmt(FmtMode::Check),
        "fmt" => Task::Fmt(FmtMode::Write),
        "test" => Task::Test,
        "test-boot" => Task::TestBoot(profile),
        "boot-check" => Task::BootCheck(profile),
        "degraded-check" => Task::DegradedCheck(profile),
        "supply-chain" => Task::SupplyChain,
        "verify-vendor" => Task::VerifyVendor,
        "model-check" => Task::ModelCheck,
        "debug" => Task::Debug,
        "ci" => Task::Ci,
        "clean" if options.all => Task::Clean(CleanScope::Everything),
        "clean" => Task::Clean(CleanScope::Build),
        other => bail!("unknown task `{other}`"),
    })
}
