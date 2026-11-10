//! Locating and launching QEMU.

mod boot_check;
mod debug;
mod run_files;
mod selftest;
mod serial;
mod storage;

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use panic_record::{COMPLETE_MARKER, OPEN_PREFIX};

use crate::paths::Paths;
use crate::qmp::{NOT_CANCELLED, Qmp};

pub use self::boot_check::{
    observe_degraded, observe_no_x2apic, observe_unisolated, run_boot_check,
};
pub(crate) use self::debug::DebugMachine;
use self::run_files::*;
pub(crate) use self::selftest::measure_performance;
use self::selftest::*;
pub use self::selftest::{observe_selftest, run_selftest};
pub(crate) use self::serial::PerformanceSample;
use self::serial::*;
pub use self::serial::{DegradedOutcome, SelftestOutcome};
pub(crate) use self::storage::nvme_path;
use self::storage::*;
pub use self::storage::{
    NVME_BLOCK_BYTES, NVME_SIGNED_BLOCKS, NVME_VOLUME_SECTOR, nvme_image, nvme_signature,
};

/// Default install location on Windows, checked before `PATH`.
const WINDOWS_DEFAULT: &str = r"C:\Program Files\qemu";

/// The q35 machine type shared by both runners.
pub(crate) const MACHINE_TYPE: &str = "pc-q35-9.2";

/// Whether the fixed machine presents the DMA-remapping unit storage requires.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Remapping {
    /// The ordinary machine: an Intel unit ahead of every PCI device (D-357).
    Present,
    /// The degraded machine that proves storage fails closed without one.
    Absent,
}

/// The remapping unit owns both DMA and interrupt delivery. Its empty IR table
/// blocks every PCI interrupt while caching mode makes the unit cache
/// not-present DMA entries, so every mapping change must be invalidated.
const IOMMU_DEVICE: [&str; 2] = ["-device", "intel-iommu,intremap=on,eim=on,caching-mode=on"];

/// QEMU 9.2 deliberately emits a scoped DRHD even though its one emulated
/// unit intercepts every PCI DMA request. The release boundary does not infer
/// catch-all authority from that implementation detail, so the fixed test
/// platform supplies a separate checksummed attestation naming the real unit.
const QEMU_DMA_COVERAGE_BYTES: usize = 48;

/// The fixed machine profile, with or without the remapping unit.
///
/// Pins the CPU, TCG, and ambient configuration so protection checks cannot
/// pass under a weaker or different execution model (D-18). Its exact
/// CPU-resident hypervisor leaf authenticates D-390's otherwise writable ACPI
/// handoff bytes. The unit comes ahead of the bridge and the storage devices,
/// which QEMU requires of it.
fn machine_args(remapping: Remapping) -> Vec<&'static str> {
    let mut arguments = vec![
        "-no-user-config",
        "-accel",
        "tcg",
        "-machine",
        MACHINE_TYPE,
        "-cpu",
        "max,hypervisor=on,tcg-cpuid=on",
        "-m",
        "512M",
    ];
    if remapping == Remapping::Present {
        arguments.extend(IOMMU_DEVICE);
    }
    // Provide a downstream bus so a bus-zero-only scan fails as it would on
    // VMware's bridged topology (D-168).
    arguments.extend(["-device", "pci-bridge,id=behind,chassis_nr=1"]);
    arguments
}

fn validate_run_arguments(extra: &[OsString]) -> Result<()> {
    let mut index = 0;
    while index < extra.len() {
        let argument = extra[index]
            .to_str()
            .context("QEMU arguments must be UTF-8")?;
        let (option, inline) = argument
            .split_once('=')
            .map_or((argument, None), |(name, value)| (name, Some(value)));
        if !matches!(option, "-d" | "-D" | "-display" | "--display") {
            bail!("QEMU argument {argument:?} conflicts with the fixed TCG machine profile");
        }
        if let Some(value) = inline {
            ensure!(
                !value.is_empty(),
                "QEMU diagnostic argument {argument:?} has no value"
            );
            index += 1;
            continue;
        }
        let value = extra
            .get(index + 1)
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty() && !value.starts_with('-'))
            .with_context(|| format!("QEMU diagnostic argument {argument:?} has no value"))?;
        let _ = value;
        index += 2;
    }
    Ok(())
}

/// Subdirectories of the QEMU installation that may hold firmware images.
const FIRMWARE_DIRS: &[&str] = &["share", "share/qemu", "share/OVMF", "."];

/// The UEFI firmware image, mapped read-only.
const CODE_FIRMWARE: &str = "edk2-x86_64-code.fd";

/// Template for the writable UEFI variable store. The i386 naming is genuine
/// QEMU packaging, not a mistake.
const VARS_FIRMWARE: &str = "edk2-i386-vars.fd";

/// Resolved emulator and firmware inputs used by every QEMU runner.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Installation {
    pub(crate) emulator: PathBuf,
    pub(crate) code_firmware: PathBuf,
    pub(crate) vars_firmware: PathBuf,
}

/// Append the platform's executable suffix to `stem`.
pub fn exe_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

/// Find `exe` in `dirs`.
pub fn which_in<I: IntoIterator<Item = PathBuf>>(dirs: I, exe: &str) -> Option<PathBuf> {
    dirs.into_iter()
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

/// Locate the exact QEMU executable.
///
/// `QEMU` names the emulator binary outright; `QEMU_DIR` names the installation
/// it lives in. Both win over the default location and over `PATH`.
fn locate(
    binary: Option<PathBuf>,
    directory: Option<PathBuf>,
    search: impl IntoIterator<Item = PathBuf>,
) -> Result<PathBuf> {
    let emulator = exe_name("qemu-system-x86_64");
    if let Some(binary) = binary {
        let refusal = format!("QEMU names {}, which is not a file", binary.display());
        return require_emulator(binary, refusal);
    }
    if let Some(dir) = directory {
        let refusal = format!(
            "QEMU_DIR names {}, which holds no {emulator}",
            dir.display()
        );
        return require_emulator(dir.join(&emulator), refusal);
    }

    let default = PathBuf::from(WINDOWS_DEFAULT);
    if default.join(&emulator).is_file() {
        return Ok(default.join(&emulator));
    }

    match which_in(search, &emulator) {
        Some(found) => Ok(found),
        None => bail!(
            "QEMU not found: looked for {emulator} in {WINDOWS_DEFAULT} and on PATH; \
             set QEMU to the executable or QEMU_DIR to its installation"
        ),
    }
}

fn require_emulator(path: PathBuf, refusal: String) -> Result<PathBuf> {
    ensure!(path.is_file(), refusal);
    Ok(path)
}

/// An FHS install keeps the emulator in `<prefix>/bin` and its firmware under
/// `<prefix>/share`, so that prefix is searched after the emulator's directory.
fn firmware(qemu_dir: &Path, name: &str) -> Result<PathBuf> {
    let prefix = qemu_dir.parent().filter(|_| qemu_dir.ends_with("bin"));
    let candidates: Vec<PathBuf> = std::iter::once(qemu_dir)
        .chain(prefix)
        .flat_map(|root| {
            FIRMWARE_DIRS
                .iter()
                .map(move |dir| root.join(dir).join(name))
        })
        .collect();
    candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .cloned()
        .with_context(|| {
            let searched: Vec<String> = candidates
                .iter()
                .map(|candidate| candidate.display().to_string())
                .collect();
            format!(
                "UEFI firmware {name} not found; QEMU needs the edk2 images {CODE_FIRMWARE} \
                 and {VARS_FIRMWARE}. Searched:\n  {}",
                searched.join("\n  ")
            )
        })
}

/// Resolve the exact emulator and pristine firmware inputs a boot will use.
pub(crate) fn installation() -> Result<Installation> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    installation_with(
        std::env::var_os("QEMU").map(PathBuf::from),
        std::env::var_os("QEMU_DIR").map(PathBuf::from),
        std::env::split_paths(&path),
    )
}

fn installation_with(
    binary: Option<PathBuf>,
    directory: Option<PathBuf>,
    search: impl IntoIterator<Item = PathBuf>,
) -> Result<Installation> {
    let emulator = std::path::absolute(locate(binary, directory, search)?)
        .context("resolving the QEMU executable path")?;
    let directory = emulator
        .parent()
        .with_context(|| format!("{} has no parent directory", emulator.display()))?;
    Ok(Installation {
        code_firmware: firmware(directory, CODE_FIRMWARE)?,
        vars_firmware: firmware(directory, VARS_FIRMWARE)?,
        emulator,
    })
}

/// Boot `image` in QEMU with non-profile `extra` arguments.
pub fn run(paths: &Paths, image: &Path, extra: &[OsString]) -> Result<()> {
    validate_run_arguments(extra)?;
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
    let coverage = RunDmaCoverage::create(paths.target_dir(), Remapping::Present)?;

    println!("[xtask] launching QEMU (serial on stdio, close the window to exit)");

    let status = Command::new(&installation.emulator)
        .current_dir(vars.directory())
        .args(machine_args(Remapping::Present))
        .args(coverage.arguments())
        .args(["-serial", "stdio"])
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .args(extra)
        .status()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    // A non-zero status here usually means the user closed the window, which is
    // how you quit QEMU — reporting it as a build failure would be a lie.
    if !status.success() {
        println!("[xtask] QEMU exited with {status}");
    }
    Ok(())
}

/// An optimization-stable payload marker shared with `kernel/src/selftest.rs`.
///
/// The formatted verdict is not a sound build-identity marker: optimized Rust
/// may split its literal around formatting placeholders. This fixed record is
/// emitted only by a self-test kernel and remains one contiguous byte string.
pub(crate) const SELFTEST_IMAGE_MARKER: &str = "selftest: image marker v1";

/// The serial verdict contract shared with `kernel/src/selftest.rs`.
pub(crate) const SELFTEST_VERDICT_PREFIX: &str = "selftest: verdict ";
/// What the kernel prints around a failing check's name, from
/// `kernel/src/selftest.rs`'s `error!("selftest: {name} FAILED")`.
const SELFTEST_CHECK_PREFIX: &str = "selftest: ";
const SELFTEST_CHECK_FAILED: &str = " FAILED";
const SELFTEST_VERDICT_VERSION: &str = "v1";

const SELFTEST_QEMU_TCG_ISOLATION_MARKER: &str =
    "cpu: user isolation paired roots: forced for self-test; shared proof retained: qemu_tcg";
const SHIPPED_QEMU_TCG_ISOLATION_MARKER: &str =
    "cpu: user isolation shared kernel mappings: qemu_tcg";
const STRICT_DMA_HANDOFF_MARKER: &str = "iommu: entry DMA fence policy strict";
/// Checks the self-test kernel must report, held independently of its own
/// tally so a deleted or skipped one turns the verdict red.
/// Changes require mutation evidence (D-28, D-119).
const EXPECTED_SELFTEST_CHECKS: usize = 296;

/// Prefix of the strict performance/resource record emitted by self-test.
const BASELINE_PREFIX: &str = "selftest: baseline ";

/// Prefix and bounds of the painted NVMe candidate-stack record.
const NVME_STACK_PREFIX: &str = "selftest: nvme-stack ";
const NVME_STACK_CAPACITY: u64 = 32 * 1024;
const NVME_STACK_MIN_HEADROOM: u64 = 2 * 4096;

/// The worker has submitted an NVMe command but remains preemptible.
const STORAGE_DELAY_MARKER: &str = "selftest: awaiting storage-delay keystroke";

/// Line the self-test kernel prints immediately before its bounded wait for
/// an injected keystroke. Kept in step with `kernel/src/selftest.rs`.
const READY_MARKER: &str = "selftest: awaiting injected keystroke";

/// Line the self-test prints before its second wait, with the controller made
/// to translate. Kept in step with `kernel/src/selftest.rs`.
///
/// A second round reaches the distinct set-1 translation path (D-179).
const TRANSLATED_MARKER: &str = "selftest: awaiting translated keystroke";

/// Line the shipped kernel prints once it has finished booting and is doing
/// the one thing it does. Kept in step with `kernel/src/main.rs`.
const SETTLED_MARKER: &str = "kernel: reading the keyboard, uptime ";

/// Evidence emitted only after the shipped program calls from saved CPL 3.
pub(crate) const SHIPPED_USER_EXECUTION_MARKER: &str = "user: ring 3 call observed";

/// Proof that the shipped selector retired QEMU's first storage candidate.
pub const NVME_FALLBACK_MARKER: &str = "nvme: candidate 1 retired cleanly; trying next";

const SHIPPED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[
    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),
    RequiredLogRecord::info(SHIPPED_QEMU_TCG_ISOLATION_MARKER),
    RequiredLogRecord::info(NVME_FALLBACK_MARKER),
    RequiredLogRecord::info(
        "keyboard: PCI interrupt isolation keeps every io-apic line masked; polling",
    ),
    RequiredLogRecord::info("keyboard: polled every 10 ms"),
    RequiredLogRecord::info(SHIPPED_USER_EXECUTION_MARKER),
];

/// A shipped kernel must accept this input without unmasking legacy IRQ1.
const SHIPPED_CHALLENGE_QCODE: &str = "a";
const SHIPPED_CHALLENGE_TIMEOUT: Duration = Duration::from_secs(5);
const SHIPPED_CHALLENGE_WINDOW: Duration = Duration::from_millis(250);

/// What a kernel that has given up says. Watched for rather than inferred
/// from an exit code: a panicking kernel halts, it does not exit.
/// How a red self-test reports itself, and how a dead one does.
///
/// Named rather than written twice because `cargo xtask mutate` has to tell a
/// kernel that *noticed* a deliberate defect from one that would not build —
/// the first is the mutation being caught and the second proves nothing
/// (D-119).
pub const SELFTEST_FAILURE: &str = "self-test reported failures - see the log above";
pub const PANIC_FAILURE: &str = "the self-test kernel panicked: see the log above";

pub(crate) fn canonical_decimal(text: &str) -> Option<u64> {
    if text.is_empty()
        || (text.len() > 1 && text.starts_with('0'))
        || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    <u64 as core::str::FromStr>::from_str(text).ok()
}

/// What the kernel must say when it finds no x2APIC.
///
/// The refusal D-33 chose and D-90 made a gate check: xAPIC needs page tables
/// that do not exist when the APIC is brought up, so there is no fallback —
/// only a diagnosis.
const NO_X2APIC_DIAGNOSIS: &str = "local apic: this processor has no x2apic mode, and xapic needs page tables; interrupts stay off";
const NO_X2APIC_RECORD: RequiredLogRecord = RequiredLogRecord::warning(NO_X2APIC_DIAGNOSIS);

/// The entry boundary must stop before memory initialization when even one
/// requester cannot be fenced (D-390).
pub const NO_PCI_HANDOFF_PANIC: &str =
    "DMA-safe boot handoff refused: DMA handoff fence: PCI configuration is unavailable";

/// The hardware-free namespace witness targeted by the PW-207 mutation.
pub const DEGRADED_FILESYSTEM_MARKER: &str = "tmpfs: 1 file(s) and 1 directory(ies) at the root";

/// Both device names must resolve through devfs without a bus.
const DEGRADED_STORAGE_MARKER: &str = "devfs: storage is a block device";
const DEGRADED_CONSOLE_MARKER: &str = "devfs: console is a character device";

/// Optional hardening is absent, while mandatory WP and NX remain active.
const DEGRADED_PROTECTIONS_MARKER: &str = "cpu: protections wp nx";

/// The degraded CPU independently proves that extended state is unavailable.
pub const DEGRADED_XSTATE_MARKER: &str = "cpu: extended state unavailable; ring 3 stays closed";

/// Missing x2APIC leaves interrupts unadmitted, the first user-launch veto.
pub const DEGRADED_USER_MARKER: &str =
    "user: no program: maskable interrupts are not admitted; ring 3 is closed";

const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[
    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),
    RequiredLogRecord::info(SHIPPED_QEMU_TCG_ISOLATION_MARKER),
    NO_X2APIC_RECORD,
    RequiredLogRecord::info(DEGRADED_FILESYSTEM_MARKER),
    RequiredLogRecord::info(DEGRADED_STORAGE_MARKER),
    RequiredLogRecord::info(DEGRADED_CONSOLE_MARKER),
    RequiredLogRecord::info(DEGRADED_PROTECTIONS_MARKER),
    RequiredLogRecord::warning(DEGRADED_XSTATE_MARKER),
    RequiredLogRecord::warning(DEGRADED_USER_MARKER),
];

/// What a storage candidate says when no remapping unit can isolate it (D-357).
pub const NO_ISOLATION_MARKER: &str = "storage: no DMA isolation";

/// With every candidate refused, selection ends with no controller.
pub const NO_USABLE_CONTROLLER_MARKER: &str = "nvme: no usable controller";

/// The unisolated boot must refuse every candidate and still settle; the
/// first candidate's clean retirement proves the refusal path returns the
/// lease exactly.
const UNISOLATED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[
    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),
    RequiredLogRecord::info(SHIPPED_QEMU_TCG_ISOLATION_MARKER),
    RequiredLogRecord::warning(NO_ISOLATION_MARKER),
    RequiredLogRecord::info(NVME_FALLBACK_MARKER),
    RequiredLogRecord::info(NO_USABLE_CONTROLLER_MARKER),
];

#[cfg(test)]
mod tests;
