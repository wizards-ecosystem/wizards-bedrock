//! Locating and launching QEMU.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::paths::{self, Paths};
use crate::qmp::Qmp;

/// Default install location on Windows, checked before `PATH`.
const WINDOWS_DEFAULT: &str = r"C:\Program Files\qemu";

/// The machine both runners boot.
///
/// `-cpu max` rather than QEMU's default `qemu64`, which exposes neither SMEP,
/// SMAP nor UMIP: the kernel would switch its security baseline on against a
/// CPU that cannot enforce it, and the check that it did would pass vacuously.
/// The VMware target of D-18 and the real machine both have all three.
pub(crate) const MACHINE_TYPE: &str = "pc-q35-9.2";

const MACHINE: &[&str] = &[
    "-machine",
    MACHINE_TYPE,
    "-cpu",
    "max",
    "-m",
    "512M",
    // A PCI-to-PCI bridge, and the bus behind it. q35 as QEMU ships it has
    // none, so a kernel that scanned only bus zero would pass every check
    // while being unable to find a controller on a real machine — VMware puts
    // functions behind `pciBridge0`-`pciBridge7`, and D-18 makes VMware the
    // phase's definition of done. The check that enumeration *descends* needs
    // somewhere to descend to (D-168).
    "-device",
    "pci-bridge,id=behind,chassis_nr=1",
];

/// Subdirectories of the QEMU installation that may hold firmware images.
const FIRMWARE_DIRS: &[&str] = &["share", "share/qemu", "share/OVMF", "."];

/// The UEFI firmware image, mapped read-only.
const CODE_FIRMWARE: &str = "edk2-x86_64-code.fd";

/// Template for the writable UEFI variable store. The i386 naming is genuine
/// QEMU packaging, not a mistake.
const VARS_FIRMWARE: &str = "edk2-i386-vars.fd";

/// Resolved emulator and firmware inputs used by every QEMU runner.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Locate the QEMU installation directory.
///
/// `QEMU` names the emulator binary outright; `QEMU_DIR` names the installation
/// it lives in. Both win over the default location and over `PATH`.
pub fn locate() -> Result<PathBuf> {
    if let Some(binary) = std::env::var_os("QEMU").map(PathBuf::from) {
        return binary
            .parent()
            .map(Path::to_path_buf)
            .with_context(|| format!("QEMU={} has no parent directory", binary.display()));
    }
    if let Some(dir) = std::env::var_os("QEMU_DIR").map(PathBuf::from) {
        return Ok(dir);
    }

    let emulator = exe_name("qemu-system-x86_64");
    let default = PathBuf::from(WINDOWS_DEFAULT);
    if default.join(&emulator).is_file() {
        return Ok(default);
    }

    let path = std::env::var_os("PATH").unwrap_or_default();
    match which_in(std::env::split_paths(&path), &emulator) {
        Some(found) => found
            .parent()
            .map(Path::to_path_buf)
            .with_context(|| format!("{} has no parent directory", found.display())),
        None => bail!("QEMU not found — set QEMU_DIR, or put {emulator} on PATH"),
    }
}

fn firmware(qemu_dir: &Path, name: &str) -> Result<PathBuf> {
    FIRMWARE_DIRS
        .iter()
        .map(|dir| qemu_dir.join(dir).join(name))
        .find(|candidate| candidate.is_file())
        .with_context(|| {
            format!(
                "UEFI firmware {name} not found under {}",
                qemu_dir.display()
            )
        })
}

/// Resolve the exact emulator and pristine firmware inputs a boot will use.
pub(crate) fn installation() -> Result<Installation> {
    let directory = locate()?;
    Ok(Installation {
        emulator: directory.join(exe_name("qemu-system-x86_64")),
        code_firmware: firmware(&directory, CODE_FIRMWARE)?,
        vars_firmware: firmware(&directory, VARS_FIRMWARE)?,
    })
}

/// Boot `image` in QEMU, forwarding `extra` arguments verbatim.
pub fn run(paths: &Paths, image: &Path, extra: &[OsString]) -> Result<()> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;

    println!("[xtask] launching QEMU (serial on stdio, close the window to exit)");

    let status = Command::new(&installation.emulator)
        .args(MACHINE)
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

/// One paused, private QEMU machine owned by a debugger invocation.
pub(crate) struct DebugMachine {
    child: Option<Child>,
    endpoint: String,
    _vars: RunVars,
}

impl DebugMachine {
    /// Start the machine and return the dynamically allocated GDB endpoint.
    pub(crate) fn start(paths: &Paths, image: &Path) -> Result<Self> {
        let installation = installation()?;
        let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
        let monitor_listener = control_listener("debug monitor")?;
        let monitor_endpoint = control_endpoint(&monitor_listener, MONITOR_CLIENT_SUFFIX)?;

        let mut child = Command::new(&installation.emulator)
            .args(MACHINE)
            .args(["-S", "-display", "none", "-serial", "null", "-no-reboot"])
            .args(["-monitor", &monitor_endpoint])
            .args(drive_file(
                "if=pflash,format=raw,readonly=on",
                &installation.code_firmware,
            ))
            .args(drive_file("if=pflash,format=raw", vars.path()))
            .args(snapshot_drive_file("format=raw", image))
            .args(storage(paths.root())?)
            .spawn()
            .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

        let endpoint = (|| -> Result<String> {
            let deadline = Instant::now() + DEBUG_CONTROL_TIMEOUT;
            let mut monitor = Monitor::accept(&monitor_listener, deadline)
                .context("accepting the paused machine's monitor")?;
            monitor
                .command("gdbserver tcp:127.0.0.1:0\n")
                .context("starting QEMU's dynamic GDB server")?;
            let report = monitor
                .command("info chardev\n")
                .context("querying QEMU's dynamic GDB endpoint")?;
            parse_debug_endpoint(&report)
        })();

        match endpoint {
            Ok(endpoint) => Ok(Self {
                child: Some(child),
                endpoint,
                _vars: vars,
            }),
            Err(error) => {
                let cleanup = stop_qemu(&mut child);
                match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => {
                        Err(anyhow!("{error:#}; QEMU cleanup also failed: {cleanup:#}"))
                    }
                }
            }
        }
    }

    /// Loopback address for GDB's `target remote` command.
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Stop and reap the machine before its private state is removed.
    pub(crate) fn stop(&mut self) -> Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        stop_qemu(child)?;
        self.child = None;
        Ok(())
    }
}

impl Drop for DebugMachine {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = stop_qemu(child);
        }
    }
}

const DEBUG_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const DEBUG_ENDPOINT_PREFIX: &str = "tcp:127.0.0.1:";

fn parse_debug_endpoint(report: &str) -> Result<String> {
    let line = report
        .lines()
        .find(|line| line.trim_start().starts_with("gdb:"))
        .context("QEMU reported no GDB chardev")?;
    let tail = line
        .split_once(DEBUG_ENDPOINT_PREFIX)
        .map(|(_, tail)| tail)
        .context("QEMU's GDB chardev is not bound to IPv4 loopback")?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    let port = digits
        .parse::<u16>()
        .context("QEMU reported an invalid GDB port")?;
    if port == 0 {
        bail!("QEMU did not allocate a GDB port");
    }
    Ok(format!("127.0.0.1:{port}"))
}

fn drive_file(options: &str, path: &Path) -> [OsString; 2] {
    let escaped = path.to_string_lossy().replace(',', ",,");
    [
        OsString::from("-drive"),
        OsString::from(format!("{options},file={escaped}")),
    ]
}

fn snapshot_drive_file(options: &str, path: &Path) -> [OsString; 2] {
    drive_file(&format!("{options},snapshot=on"), path)
}

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

struct RunVars {
    path: PathBuf,
}

impl RunVars {
    fn create(target: &Path, template: &Path) -> Result<Self> {
        let directory = target.join(paths::QEMU_RUN_DIRECTORY);
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;

        for _ in 0..1024 {
            let sequence = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!("efi-vars-{}-{sequence}.fd", std::process::id()));
            let mut destination = match OpenOptions::new().write(true).create_new(true).open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("creating {}", path.display()));
                }
            };
            let copied = (|| -> Result<()> {
                let mut source = File::open(template)
                    .with_context(|| format!("opening {}", template.display()))?;
                std::io::copy(&mut source, &mut destination).with_context(|| {
                    format!("copying {} to {}", template.display(), path.display())
                })?;
                destination
                    .sync_all()
                    .with_context(|| format!("flushing {}", path.display()))?;
                Ok(())
            })();
            if let Err(error) = copied {
                drop(destination);
                let _ = std::fs::remove_file(&path);
                return Err(error);
            }
            return Ok(Self { path });
        }
        bail!("could not reserve a private QEMU variable store")
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RunVars {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bytes of one logical block on the NVMe namespace.
///
/// QEMU's controller formats at 512 and the kernel reads what `Identify`
/// reports rather than this; the constant is here because the *content* below
/// is written a block at a time and has to agree with what the driver will
/// see.
pub const NVME_BLOCK_BYTES: usize = 512;

/// Blocks given known content.
pub const NVME_SIGNED_BLOCKS: u64 = 8;

/// The first eight bytes of block `lba`.
///
/// **Different in every block, on purpose.** A disk of zeros cannot tell a read
/// that happened from one that did not, and a disk with the same bytes
/// everywhere cannot tell a read of block three from a read of block zero — so
/// a driver that ignored the logical block address would pass. This is D-155's
/// rule applied to a corpus rather than to an argument (D-169).
pub const fn nvme_signature(lba: u64) -> u64 {
    0x5749_5A44_0000_0000 | lba
}

/// Where the FAT32 volume begins on the namespace, in sectors.
///
/// A megabyte in, which is where a partition table would put it. The reader
/// takes a base sector rather than assuming zero for exactly this reason: a
/// volume is rarely the first thing on a disk, and a reader that could not be
/// given an offset is one nothing can be put in front of (D-170).
pub const NVME_VOLUME_SECTOR: u64 = 2048;

/// Bytes of the NVMe backing file.
///
/// The first megabyte carries signed blocks the block-layer checks read, and
/// the FAT32 corpus follows it. Two things on one namespace rather than two
/// namespaces, because that is also what a real disk looks like.
const NVME_BYTES: u64 =
    NVME_VOLUME_SECTOR * NVME_BLOCK_BYTES as u64 + crate::image::corpus::VOLUME_BYTES as u64;

/// A storage controller for the machine to find, and the file behind it.
///
/// **virtio-blk is disqualified rather than deprioritised** (`012` §2): VMware
/// exposes no virtio device of any kind, so under D-18 it could never close a
/// phase, and none of the virtqueue work carries to a driver that could. NVMe
/// is the only overlap between the two boot targets with a freely published
/// specification.
///
/// The file is created here rather than by the image builder because it is a
/// property of the *machine*, not of the disk image: nothing on it is ever
/// booted, and the kernel writes nothing to it (D-169).
/// Build the namespace's contents, and say where they are.
///
/// **Both runners use this.** `012` §2's whole argument for NVMe is that it is
/// the one controller with a published specification that spans QEMU *and*
/// VMware, and a controller given only to QEMU tests half of that (D-176).
///
/// # Errors
///
/// If the file cannot be written, or the corpus cannot be built.
pub fn nvme_image(root: &Path) -> Result<std::path::PathBuf> {
    let backing = nvme_path(root);
    std::fs::create_dir_all(backing.parent().context("NVMe image has no parent")?)
        .with_context(|| format!("creating the directory for {}", backing.display()))?;
    // Written whole rather than sized and left sparse: the first blocks carry
    // a signature the block-layer checks read, and a FAT32 volume follows at
    // `NVME_VOLUME_SECTOR`.
    let mut content = vec![0u8; NVME_BYTES as usize];
    for lba in 0..NVME_SIGNED_BLOCKS {
        let at = lba as usize * NVME_BLOCK_BYTES;
        content[at..at + 8].copy_from_slice(&nvme_signature(lba).to_le_bytes());
    }
    let volume = crate::image::corpus::build().context("building the corpus volume")?;
    let at = NVME_VOLUME_SECTOR as usize * NVME_BLOCK_BYTES;
    content[at..at + volume.len()].copy_from_slice(&volume);

    let _lock = acquire_storage_lock(&backing.with_extension("lock"))?;
    let current = std::fs::read(&backing).unwrap_or_default();
    if !namespace_is_exact(&current, &content) {
        std::fs::write(&backing, &content)
            .with_context(|| format!("writing {}", backing.display()))?;
    }
    Ok(backing)
}

const STORAGE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

fn acquire_storage_lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening storage build lock {}", path.display()))?;
    let deadline = Instant::now() + STORAGE_LOCK_TIMEOUT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    bail!("another runner did not release {}", path.display());
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("locking storage build {}", path.display()));
            }
        }
    }
}

pub(crate) fn nvme_path(root: &Path) -> PathBuf {
    root.join("target")
        .join("qemu storage,private")
        .join("nvme.img")
}

fn namespace_is_exact(current: &[u8], expected: &[u8]) -> bool {
    current == expected
}

/// A storage controller for QEMU to find, and the file behind it.
fn storage(root: &Path) -> Result<Vec<OsString>> {
    let backing = nvme_image(root)?;
    let [drive_flag, drive_spec] = snapshot_drive_file("if=none,id=wizardnvm,format=raw", &backing);
    Ok(vec![
        // First in PCI enumeration order and deliberately namespace-less: the
        // kernel must retire it before selecting the usable sibling (D-199).
        OsString::from("-device"),
        OsString::from("nvme,serial=wizard-empty"),
        drive_flag,
        drive_spec,
        OsString::from("-device"),
        OsString::from("nvme,serial=wizard0,drive=wizardnvm"),
    ])
}

/// QEMU exits with `(value << 1) | 1`, so the kernel's 0x10 arrives as 33 and
/// its 0x11 as 35. Kept in step with `kernel/src/arch/x86_64/debug_exit.rs`.
const SELFTEST_PASSED: i32 = 33;
const SELFTEST_FAILED: i32 = 35;

/// The serial verdict contract shared with `kernel/src/selftest.rs`.
pub(crate) const SELFTEST_VERDICT_PREFIX: &str = "selftest: verdict ";
/// What the kernel prints around a failing check's name, from
/// `kernel/src/selftest.rs`'s `error!("selftest: {name} FAILED")`.
const SELFTEST_CHECK_PREFIX: &str = "selftest: ";
const SELFTEST_CHECK_FAILED: &str = " FAILED";
const SELFTEST_VERDICT_VERSION: &str = "v1";
/// Checks the self-test kernel must report, held independently of its own
/// tally so a deleted or skipped one turns the verdict red.
///
/// **Changing this number is the moment a mutation is owed.** `AGENTS.md` says
/// to add one whenever a check is added, and twice in one day it was not —
/// five checks landed with a hand break-test and no entry in
/// `xtask/src/mutate.rs`, by two sessions that had been citing D-119 all day.
/// The rule was not forgotten; it was never *reached*, because adding a check
/// is a thing you do inside a fix and the table is a different file. This line
/// and its twin in `kernel/src/selftest.rs` are the two you have to touch
/// either way, so the reminder goes where the hand already is (D-28, D-119).
const EXPECTED_SELFTEST_CHECKS: usize = 215;

/// Prefix of the strict performance/resource record emitted by self-test.
const BASELINE_PREFIX: &str = "selftest: baseline ";

/// The worker has submitted an NVMe command but remains preemptible.
const STORAGE_DELAY_MARKER: &str = "selftest: awaiting storage-delay keystroke";

/// Line the self-test kernel prints immediately before its bounded wait for
/// an injected keystroke. Kept in step with `kernel/src/selftest.rs`.
const READY_MARKER: &str = "selftest: awaiting injected keystroke";

/// Line the self-test prints before its second wait, with the controller made
/// to translate. Kept in step with `kernel/src/selftest.rs`.
///
/// Two rounds because the two scancode sets are two code paths, and the second
/// one had never been exercised on any machine the harness could inject into
/// (D-179).
const TRANSLATED_MARKER: &str = "selftest: awaiting translated keystroke";

/// Line the shipped kernel prints once it has finished booting and is doing
/// the one thing it does. Kept in step with `kernel/src/main.rs`.
const SETTLED_MARKER: &str = "kernel: reading the keyboard";

/// Proof that the shipped selector retired QEMU's first storage candidate.
pub const NVME_FALLBACK_MARKER: &str = "nvme: candidate 1 retired cleanly; trying next";

/// A shipped kernel must take this interrupt after announcing its idle loop.
const SHIPPED_CHALLENGE_QCODE: &str = "a";
const SHIPPED_CHALLENGE_TIMEOUT: Duration = Duration::from_secs(5);

/// What a kernel that has given up says. Watched for rather than inferred
/// from an exit code: a panicking kernel halts, it does not exit.
const PANIC_MARKER: &str = "KERNEL PANIC";

/// How a red self-test reports itself, and how a dead one does.
///
/// Named rather than written twice because `cargo xtask mutate` has to tell a
/// kernel that *noticed* a deliberate defect from one that would not build —
/// the first is the mutation being caught and the second proves nothing
/// (D-119).
pub const SELFTEST_FAILURE: &str = "self-test reported failures - see the log above";
pub const PANIC_FAILURE: &str = "the self-test kernel panicked — see the log above";

/// A complete, structurally valid self-test machine outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelftestOutcome {
    /// Every registered check passed.
    Passed,
    /// The final verdict failed and names each red check in report order.
    Failed(Vec<String>),
    /// The kernel stopped before a verdict and carries its exact serial panic
    /// transcript plus every check that had already gone red.
    Panicked {
        /// The panic line and its continuation.
        transcript: String,
        /// Named failures printed before the terminal panic.
        failed: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerdictStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelftestVerdict {
    status: VerdictStatus,
    passed: usize,
    failed: usize,
    expected: usize,
}

/// One guest-side foundation measurement from a clean self-test boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PerformanceSample {
    pub(crate) boot_ns: u64,
    pub(crate) frames_free: u64,
    pub(crate) heap_slab: u64,
    pub(crate) heap_large: u64,
    pub(crate) heap_live: u64,
    pub(crate) handles_committed: u64,
    pub(crate) handles_live: u64,
    pub(crate) trap_ticks: u64,
    pub(crate) storage_bytes: u64,
    pub(crate) storage_ticks: u64,
    pub(crate) counter_hz: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SerialObservation {
    verdict: Option<SelftestVerdict>,
    baseline: Option<PerformanceSample>,
    /// Names of the checks that reported `FAILED`, in the order printed.
    ///
    /// Recorded so a failure can say *which* check went red rather than only
    /// that one did. `cargo xtask mutate` is the caller that needs it: a
    /// mutation reported "caught" because some check failed is a weaker claim
    /// than one caught because *its* check failed, and the two came apart in
    /// the table — mutation 4 reddens mutation 3's check, so deleting 4's own
    /// target would leave it printing "caught" for ever (D-134).
    failed: Vec<String>,
    /// The serial panic line and any continuation the kernel printed.
    panic: Option<String>,
}

/// The check name in a `selftest: <name> FAILED` line, if it is one.
///
/// The line is trimmed first, and that is not tidiness: serial output arrives
/// with a trailing carriage return, so matching the suffix against the raw
/// line finds nothing. The first strict sweep reported eighteen survivors on boots that
/// had reddened exactly the right check, because of that one carriage return
/// (D-134).
fn parse_failed_check(line: &str) -> Option<&str> {
    let line = line.trim_end();
    let at = line.find(SELFTEST_CHECK_PREFIX)?;
    line[at + SELFTEST_CHECK_PREFIX.len()..].strip_suffix(SELFTEST_CHECK_FAILED)
}

/// The keys the harness injects, in order, once the marker appears.
///
/// `a` is what the keystroke check waits for. The two after it are the
/// editing keys, which reached a user before they reached a test: backspace
/// typed nothing at all, and Enter had no visible effect because the console
/// drew no cursor. Both are checked on the machine now (D-51).
const INJECTED_QCODES: &[&str] = &["a", "backspace", "ret"];

/// The key used while the storage worker is held inside a command.
const STORAGE_DELAY_QCODES: &[&str] = &["c"];

/// The second round's keystroke, pressed once the controller is translating.
///
/// **`b`, and never `a`.** One key, because the round is about the numbering
/// the bytes arrive in rather than the editing path the first round covers —
/// but it has to be a key the first round did not press, or an `a` still
/// sitting in the queue satisfies the wait and the set 1 tables are never
/// consulted. Which is what happened: the first version of this round pressed
/// `a` twice and stayed green with the set 1 code for `a` deliberately
/// broken (D-155, D-179).
const TRANSLATED_QCODES: &[&str] = &["b"];

fn parse_selftest_verdict(line: &str) -> Option<Result<SelftestVerdict>> {
    let start = line.find(SELFTEST_VERDICT_PREFIX)?;
    let mut fields = line[start + SELFTEST_VERDICT_PREFIX.len()..].split_whitespace();
    let version = fields.next();
    let status = fields.next();
    let passed = fields.next();
    let failed = fields.next();
    let expected = fields.next();

    let parsed = (|| {
        if version != Some(SELFTEST_VERDICT_VERSION) {
            bail!("unsupported self-test verdict version");
        }
        let status = match status {
            Some("PASS") => VerdictStatus::Pass,
            Some("FAIL") => VerdictStatus::Fail,
            _ => bail!("invalid self-test verdict status"),
        };
        if fields.next().is_some() {
            bail!("trailing fields in self-test verdict");
        }

        Ok(SelftestVerdict {
            status,
            passed: parse_verdict_count(passed, "passed=")?,
            failed: parse_verdict_count(failed, "failed=")?,
            expected: parse_verdict_count(expected, "expected=")?,
        })
    })();
    Some(parsed)
}

fn parse_verdict_count(field: Option<&str>, name: &str) -> Result<usize> {
    field
        .and_then(|field| field.strip_prefix(name))
        .context("missing self-test verdict count")?
        .parse()
        .context("invalid self-test verdict count")
}

fn parse_baseline_value(field: Option<&str>, name: &str) -> Result<u64> {
    field
        .and_then(|field| field.strip_prefix(name))
        .with_context(|| format!("missing baseline field {name}"))?
        .parse()
        .with_context(|| format!("invalid baseline field {name}"))
}

fn parse_performance_sample(line: &str) -> Option<Result<PerformanceSample>> {
    let start = line.find(BASELINE_PREFIX)?;
    let mut fields = line[start + BASELINE_PREFIX.len()..].split_whitespace();
    let parsed = (|| {
        if fields.next() != Some("v1") {
            bail!("unsupported foundation baseline version");
        }
        let sample = PerformanceSample {
            boot_ns: parse_baseline_value(fields.next(), "boot_ns=")?,
            frames_free: parse_baseline_value(fields.next(), "frames_free=")?,
            heap_slab: parse_baseline_value(fields.next(), "heap_slab=")?,
            heap_large: parse_baseline_value(fields.next(), "heap_large=")?,
            heap_live: parse_baseline_value(fields.next(), "heap_live=")?,
            handles_committed: parse_baseline_value(fields.next(), "handles_committed=")?,
            handles_live: parse_baseline_value(fields.next(), "handles_live=")?,
            trap_ticks: parse_baseline_value(fields.next(), "trap_ticks=")?,
            storage_bytes: parse_baseline_value(fields.next(), "storage_bytes=")?,
            storage_ticks: parse_baseline_value(fields.next(), "storage_ticks=")?,
            counter_hz: parse_baseline_value(fields.next(), "counter_hz=")?,
        };
        if fields.next().is_some() {
            bail!("trailing fields in foundation baseline");
        }
        Ok(sample)
    })();
    Some(parsed)
}

fn observe_selftest_line(observation: &mut SerialObservation, line: &str) -> Result<()> {
    if let Some(panic) = &mut observation.panic {
        panic.push('\n');
        panic.push_str(line.trim_end());
        return Ok(());
    }
    if line.contains(PANIC_MARKER) {
        observation.panic = Some(line.trim_end().to_string());
        return Ok(());
    }
    if let Some(name) = parse_failed_check(line) {
        observation.failed.push(name.to_string());
        return Ok(());
    }
    if let Some(sample) = parse_performance_sample(line) {
        let sample = sample?;
        if observation.baseline.replace(sample).is_some() {
            bail!("the self-test kernel printed more than one foundation baseline");
        }
        return Ok(());
    }
    let Some(verdict) = parse_selftest_verdict(line) else {
        return Ok(());
    };
    let verdict = verdict?;
    if observation.verdict.replace(verdict).is_some() {
        bail!("the self-test kernel printed more than one final verdict");
    }
    Ok(())
}

fn correlate_selftest(
    exit_code: Option<i32>,
    observation: SerialObservation,
) -> Result<SelftestOutcome> {
    if let Some(transcript) = observation.panic {
        return Ok(SelftestOutcome::Panicked {
            transcript,
            failed: observation.failed,
        });
    }
    let verdict = observation
        .verdict
        .context("QEMU exited without a serial self-test verdict")?;
    if verdict.expected != EXPECTED_SELFTEST_CHECKS {
        bail!(
            "self-test expected {} checks, harness requires {EXPECTED_SELFTEST_CHECKS}",
            verdict.expected
        );
    }
    let checked = verdict
        .passed
        .checked_add(verdict.failed)
        .context("self-test verdict count overflowed")?;
    if checked != verdict.expected || verdict.expected == 0 {
        bail!(
            "self-test verdict counted {checked} checks, expected {}",
            verdict.expected
        );
    }

    let counts_pass = verdict.failed == 0 && verdict.passed == verdict.expected;
    if (verdict.status == VerdictStatus::Pass) != counts_pass {
        bail!("self-test serial status contradicts its counts");
    }
    if observation.failed.len() != verdict.failed {
        bail!(
            "self-test verdict counted {} failures but named {}",
            verdict.failed,
            observation.failed.len()
        );
    }

    match (exit_code, verdict.status) {
        (Some(SELFTEST_PASSED), VerdictStatus::Pass) => Ok(SelftestOutcome::Passed),
        (Some(SELFTEST_FAILED), VerdictStatus::Fail) => {
            Ok(SelftestOutcome::Failed(observation.failed))
        }
        (Some(SELFTEST_PASSED | SELFTEST_FAILED), _) => {
            bail!("self-test serial verdict contradicts its debug-exit status")
        }
        (Some(other), _) => bail!("QEMU exited with {other}; debug-exit gave no verdict"),
        (None, _) => bail!("QEMU was terminated by a signal"),
    }
}

/// Boot `image` headless, wait for the kernel's verdict, and report it.
///
/// Report where the CPU was when the deadline ran out.
///
/// A hung kernel prints nothing, and a kernel that prints nothing is the one
/// failure that cannot be read from the log. The monitor knows the
/// instruction pointer and can disassemble around it, which is enough to name
/// the loop: the interrupt flag in `RFLAGS` says whether it can ever end.
fn report_hang(listener: &TcpListener) {
    let Ok(mut monitor) = Monitor::accept(listener, Instant::now() + MONITOR_TIMEOUT) else {
        println!("[xtask] no monitor: where the kernel hung is unknown");
        return;
    };
    let Ok(state) = monitor.command("info registers\n") else {
        println!("[xtask] no monitor: where the kernel hung is unknown");
        return;
    };
    for line in state
        .lines()
        .filter(|line| line.contains("RIP=") || line.contains("RSP="))
    {
        println!("[xtask] hung at: {}", line.trim());
    }
    let Some(rip) = state
        .split_whitespace()
        .find_map(|word| word.strip_prefix("RIP="))
    else {
        return;
    };
    let Ok(code) = monitor.command(&format!("x/8i 0x{rip}\n")) else {
        return;
    };
    for line in code
        .lines()
        .filter(|line| line.trim_start().starts_with("0x"))
    {
        println!("[xtask] hung in:  {}", line.trim());
    }
}

struct ControlSockets {
    qmp: TcpListener,
    monitor: TcpListener,
}

impl ControlSockets {
    fn bind() -> Result<Self> {
        let qmp = control_listener("QMP")?;
        let monitor = control_listener("monitor")?;
        Ok(Self { qmp, monitor })
    }
}

fn control_listener(label: &str) -> Result<TcpListener> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .with_context(|| format!("binding the {label} loopback listener"))?;
    listener
        .set_nonblocking(true)
        .with_context(|| format!("making the {label} listener non-blocking"))?;
    Ok(listener)
}

fn control_endpoint(listener: &TcpListener, suffix: &str) -> Result<String> {
    Ok(format!(
        "tcp:{}{suffix}",
        listener
            .local_addr()
            .context("reading a control-socket address")?
    ))
}

struct Monitor {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Monitor {
    fn accept(listener: &TcpListener, deadline: Instant) -> Result<Self> {
        let stream = accept_stream_before(listener, deadline, None)?;
        stream
            .set_nonblocking(false)
            .context("switching the monitor stream to blocking")?;
        stream
            .set_read_timeout(Some(MONITOR_TIMEOUT))
            .context("setting the QEMU monitor read timeout")?;
        let reader = BufReader::new(stream.try_clone().context("cloning the monitor stream")?);
        let mut monitor = Self { stream, reader };
        monitor.read_reply().context("reading the monitor prompt")?;
        Ok(monitor)
    }

    fn command(&mut self, command: &str) -> Result<String> {
        self.stream
            .write_all(command.as_bytes())
            .context("writing a QEMU monitor command")?;
        self.read_reply()
    }

    fn read_reply(&mut self) -> Result<String> {
        let mut text = String::new();
        let mut buffer = [0u8; 4096];
        while !text.contains(MONITOR_PROMPT) {
            let read = self
                .reader
                .read(&mut buffer)
                .context("reading a QEMU monitor reply")?;
            if read == 0 {
                bail!("QEMU monitor closed before completing its reply");
            }
            text.push_str(&String::from_utf8_lossy(&buffer[..read]));
        }
        Ok(text)
    }
}

/// Long enough for a reply, short enough not to extend a failing run.
const MONITOR_TIMEOUT: Duration = Duration::from_millis(1500);

/// What the monitor prints when it wants a command.
const MONITOR_PROMPT: &str = "(qemu)";

/// Make QEMU dial the listener already owned by the harness.
const MONITOR_CLIENT_SUFFIX: &str = ",server=off";

/// A kernel that neither passes nor fails within `timeout` is a hang, which is
/// itself a failure — so the emulator is killed rather than left running.
///
/// The boot carries a QMP control socket so the harness can press a key when
/// the kernel says it is listening: the harness listens first and QEMU dials
/// out, so there is no port to guess and no connect to race. Serial output is
/// piped and forwarded line by line — both to keep it visible and to watch
/// for the marker. QMP misbehaviour is reported and abandoned, never fatal
/// here: the missing keystroke turns the kernel's own bounded check red, so
/// the verdict stays the kernel's.
pub fn run_selftest(paths: &Paths, image: &Path, timeout: Duration) -> Result<()> {
    require_selftest(observe_selftest(paths, image, timeout)?)
}

/// Boot `image` headless and preserve a valid red or panic outcome for a
/// mutation caller.
pub fn observe_selftest(paths: &Paths, image: &Path, timeout: Duration) -> Result<SelftestOutcome> {
    observe_selftest_run(paths, image, timeout, true).map(|(outcome, _)| outcome)
}

/// Boot a self-test image and require its one versioned baseline record.
pub(crate) fn measure_performance(
    paths: &Paths,
    image: &Path,
    timeout: Duration,
) -> Result<PerformanceSample> {
    let (outcome, baseline) = observe_selftest_run(paths, image, timeout, false)?;
    require_selftest(outcome)?;
    baseline.context("a clean self-test boot emitted no foundation baseline")
}

fn observe_selftest_run(
    paths: &Paths,
    image: &Path,
    timeout: Duration,
    echo_serial: bool,
) -> Result<(SelftestOutcome, Option<PerformanceSample>)> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
    let ControlSockets { qmp, monitor } = ControlSockets::bind()?;
    let qmp_endpoint = control_endpoint(&qmp, "")?;
    let monitor_endpoint = control_endpoint(&monitor, MONITOR_CLIENT_SUFFIX)?;

    println!("[xtask] booting the self-test kernel");

    let mut child = Command::new(&installation.emulator)
        .args(MACHINE)
        .args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .args(["-qmp", &qmp_endpoint])
        .args(["-monitor", &monitor_endpoint])
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    let stdout = child.stdout.take().context("taking QEMU's stdout pipe")?;
    let markers_seen = Arc::new(AtomicUsize::new(0));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let forwarder = forward_serial(
        stdout,
        Arc::clone(&markers_seen),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        echo_serial,
    );
    let deadline = Instant::now() + timeout;
    let injection_cancelled = Arc::new(AtomicBool::new(false));
    let injector = inject_on_marker(
        qmp,
        markers_seen,
        deadline,
        Arc::clone(&injection_cancelled),
    );
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for QEMU")? {
            break status;
        }
        // A panicking kernel halts rather than reporting, so the timeout
        // would otherwise be the only way it ever ends.
        if panicked.load(Ordering::Acquire) {
            stop_qemu(&mut child)?;
            let observation = finish_selftest_serial(forwarder)?;
            finish_injector(injector, &injection_cancelled)?;
            let baseline = observation.baseline;
            return correlate_selftest(None, observation).map(|outcome| (outcome, baseline));
        }
        if serial_failed.load(Ordering::Acquire) {
            stop_qemu(&mut child)?;
            finish_selftest_serial(forwarder)?;
            finish_injector(injector, &injection_cancelled)?;
            unreachable!("a flagged serial failure returned success");
        }
        if Instant::now() >= deadline {
            report_hang(&monitor);
            stop_qemu(&mut child)?;
            finish_selftest_serial(forwarder)?;
            finish_injector(injector, &injection_cancelled)?;
            bail!(
                "self-test did not finish within {} seconds",
                timeout.as_secs()
            );
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    // Trailing serial lines print before the verdict does.
    let observation = finish_selftest_serial(forwarder)?;
    finish_injector(injector, &injection_cancelled)?;

    let baseline = observation.baseline;
    correlate_selftest(status.code(), observation).map(|outcome| (outcome, baseline))
}

fn require_selftest(outcome: SelftestOutcome) -> Result<()> {
    match outcome {
        SelftestOutcome::Passed => {
            println!("[xtask] self-test passed");
            Ok(())
        }
        SelftestOutcome::Failed(failed) => {
            bail!("{SELFTEST_FAILURE}: {}", failed.join("; "))
        }
        SelftestOutcome::Panicked { transcript, .. } => {
            bail!("{PANIC_FAILURE}: {transcript}")
        }
    }
}

/// Forward every serial line to the console, flagging the ready marker.
///
/// A kernel that dies mid-line still shows its last words: the unterminated
/// remainder at end-of-stream is forwarded too.
fn forward_serial(
    stdout: ChildStdout,
    markers_seen: Arc<AtomicUsize>,
    panicked: Arc<AtomicBool>,
    serial_failed: Arc<AtomicBool>,
    echo: bool,
) -> JoinHandle<Result<SerialObservation>> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        let mut observation = SerialObservation::default();
        loop {
            line.clear();
            let read = match reader.read_until(b'\n', &mut line) {
                Ok(read) => read,
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading QEMU's serial output");
                }
            };
            if read == 0 {
                return Ok(observation);
            }
            let text = String::from_utf8_lossy(&line);
            if echo {
                // One print per line, so a message from another thread cannot
                // land between a dying kernel's last words and their newline.
                if text.ends_with('\n') {
                    print!("{text}");
                } else {
                    println!("{text}");
                }
                let _ = std::io::stdout().flush();
            }
            if text.contains(STORAGE_DELAY_MARKER)
                || text.contains(READY_MARKER)
                || text.contains(TRANSLATED_MARKER)
            {
                markers_seen.fetch_add(1, Ordering::Release);
            }
            if text.contains(PANIC_MARKER) {
                panicked.store(true, Ordering::Release);
            }
            if let Err(error) = observe_selftest_line(&mut observation, &text) {
                serial_failed.store(true, Ordering::Release);
                return Err(error).context("reading the kernel's self-test verdict");
            }
        }
    })
}

fn finish_selftest_serial(
    forwarder: JoinHandle<Result<SerialObservation>>,
) -> Result<SerialObservation> {
    forwarder
        .join()
        .map_err(|_| anyhow!("the self-test serial reader panicked"))?
}

/// Press the key on a thread of its own: accept when QEMU dials in, wait for
/// the kernel's marker, inject once, and report-and-retire on any failure —
/// the kernel's bounded wait is what turns a missing key red.
///
/// A dedicated thread rather than steps woven into the poll loop, so that no
/// QMP stall — a slow handshake, a peer dribbling bytes — can delay the loop
/// that enforces the run's deadline. Accepting and marker waits share that
/// deadline; every exit path cancels and joins the thread.
fn inject_on_marker(
    listener: TcpListener,
    markers_seen: Arc<AtomicUsize>,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let outcome = accept_stream_before(&listener, deadline, Some(&cancelled))
            .and_then(Qmp::handshake)
            .and_then(|mut qmp| {
                for (round, keys) in [
                    (1, STORAGE_DELAY_QCODES),
                    (2, INJECTED_QCODES),
                    (3, TRANSLATED_QCODES),
                ] {
                    while markers_seen.load(Ordering::Acquire) < round {
                        if cancelled.load(Ordering::Acquire) {
                            bail!("QMP injection was cancelled");
                        }
                        if Instant::now() >= deadline {
                            bail!("QMP injection reached the run deadline");
                        }
                        std::thread::sleep(POLL_INTERVAL);
                    }
                    keys.iter().try_for_each(|qcode| qmp.inject_key(qcode))?;
                }
                Ok(())
            });
        match outcome {
            Ok(()) => println!(
                "[xtask] qmp: {} keystrokes injected across three rounds",
                STORAGE_DELAY_QCODES.len() + INJECTED_QCODES.len() + TRANSLATED_QCODES.len()
            ),
            Err(reason) => {
                println!("[xtask] qmp: {reason:#}; the keystroke check will report it");
            }
        }
    })
}

/// Cancel and join the self-test injector.
fn finish_injector(injector: JoinHandle<()>, cancelled: &AtomicBool) -> Result<()> {
    cancelled.store(true, Ordering::Release);
    injector
        .join()
        .map_err(|_| anyhow!("the QMP injector panicked"))
}

fn accept_before(listener: &TcpListener, deadline: Instant) -> Result<Qmp> {
    Qmp::handshake(accept_stream_before(listener, deadline, None)?)
}

fn accept_stream_before(
    listener: &TcpListener,
    deadline: Instant,
    cancelled: Option<&AtomicBool>,
) -> Result<TcpStream> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                    bail!("QEMU control connection was cancelled");
                }
                if Instant::now() >= deadline {
                    bail!("QEMU did not open its control connection");
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(error) => return Err(error).context("accepting QEMU's QMP connection"),
        }
    }
}

/// How often to check whether QEMU has exited. Short enough that a passing run
/// is not padded noticeably, long enough not to spin.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

fn stop_qemu(child: &mut Child) -> Result<()> {
    if child
        .try_wait()
        .context("checking whether QEMU exited")?
        .is_none()
    {
        let killed = child.kill();
        if let Err(error) = killed {
            let status = child
                .try_wait()
                .context("checking QEMU after termination failed")?;
            if status.is_none() {
                return Err(error).context("terminating QEMU");
            }
        }
    }
    child.wait().context("reaping QEMU").map(drop)
}

fn irq_count(report: &str, irq: u8) -> Option<u64> {
    let mut recognized = false;
    let mut total = 0u64;
    for line in report.lines() {
        if line.contains("IRQ statistics") {
            recognized = true;
            continue;
        }
        let Some((label, count)) = line.split_once(':') else {
            continue;
        };
        let label = label.trim().strip_prefix("IRQ").unwrap_or(label.trim());
        if label.trim().parse::<u8>().ok() != Some(irq) {
            continue;
        }
        let count = count.split_whitespace().next()?.parse::<u64>().ok()?;
        total = total.checked_add(count)?;
    }
    recognized.then_some(total)
}

fn keyboard_irq_count(monitor: &mut Monitor) -> Result<u64> {
    let report = monitor
        .command("info irq\n")
        .context("querying keyboard IRQ progress")?;
    irq_count(&report, 1).context("QEMU monitor returned no IRQ statistics")
}

fn await_keyboard_irq(
    child: &mut Child,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
    monitor: &mut Monitor,
    before: u64,
) -> Result<u64> {
    let deadline = Instant::now() + SHIPPED_CHALLENGE_TIMEOUT;
    while Instant::now() < deadline {
        watch(
            child,
            panicked,
            serial_failed,
            Duration::from_millis(1),
            None,
        )?;
        let after = keyboard_irq_count(monitor)?;
        if after > before {
            return Ok(after);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    bail!("the shipped kernel did not take the injected keyboard interrupt")
}

/// Boot the image the kernel ships and require it to reach its steady state.
///
/// The self-test build exits at the debug-exit device, so nothing it does
/// covers what the shipped kernel does afterwards. The verdict here is what
/// the serial log says: the settled marker, and no panic.
pub fn run_boot_check(paths: &Paths, image: &Path, settle: Duration) -> Result<()> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
    let ControlSockets { qmp, monitor } = ControlSockets::bind()?;
    let qmp_endpoint = control_endpoint(&qmp, "")?;
    let monitor_endpoint = control_endpoint(&monitor, MONITOR_CLIENT_SUFFIX)?;

    println!("[xtask] booting the shipped kernel");

    let mut child = Command::new(&installation.emulator)
        .args(MACHINE)
        .args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(["-qmp", &qmp_endpoint])
        .args(["-monitor", &monitor_endpoint])
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    let stdout = child.stdout.take().context("taking QEMU's stdout pipe")?;
    let settled = Arc::new(AtomicBool::new(false));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let required = vec![(NVME_FALLBACK_MARKER, Arc::new(AtomicBool::new(false)))];
    let forwarder = watch_serial(
        stdout,
        Arc::clone(&settled),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        required.clone(),
    );

    // Reach the idle loop, prove a fresh device interrupt wakes it, then keep
    // watching it long enough to catch a delayed panic.
    let outcome = watch(
        &mut child,
        &panicked,
        &serial_failed,
        BOOT_TIMEOUT,
        Some(&settled),
    )
    .and_then(|()| {
        let mut monitor = Monitor::accept(&monitor, Instant::now() + SHIPPED_CHALLENGE_TIMEOUT)?;
        let before = keyboard_irq_count(&mut monitor)?;
        let mut qmp = accept_before(&qmp, Instant::now() + SHIPPED_CHALLENGE_TIMEOUT)?;
        qmp.inject_key(SHIPPED_CHALLENGE_QCODE)
            .context("injecting the shipped-kernel liveness key")?;
        let after =
            await_keyboard_irq(&mut child, &panicked, &serial_failed, &mut monitor, before)?;
        println!("[xtask] shipped liveness: keyboard IRQ count {before} -> {after}");
        Ok(())
    })
    .and_then(|()| watch(&mut child, &panicked, &serial_failed, settle, None));

    let stop_outcome = stop_qemu(&mut child);
    let serial_outcome = finish_boot_serial(forwarder);

    stop_outcome?;
    serial_outcome?;
    outcome?;
    // Consulted once more after the join. `watch` returns as soon as the
    // settle deadline passes, and `finish_boot_serial` reports only read
    // errors — so a panic printed in the last poll interval, or between the
    // deadline and the kill, was forwarded to the log and then reported as a
    // clean boot (D-105).
    if panicked.load(Ordering::Acquire) {
        anyhow::bail!("the kernel panicked during the settle window");
    }
    for (marker, seen) in required {
        if !seen.load(Ordering::Acquire) {
            bail!("the shipped kernel settled without reporting {marker:?}");
        }
    }
    println!("[xtask] shipped kernel falls back, boots, responds, and settles");
    Ok(())
}

/// How long to allow for a cold start before the marker must have appeared.
const BOOT_TIMEOUT: Duration = Duration::from_secs(45);

/// A processor with no x2APIC or optional SMEP/SMAP/UMIP hardening.
///
/// The last `-cpu` wins, so this overrides [`MACHINE`]'s.
const DEGRADED_CPU: [&str; 2] = ["-cpu", "qemu64,-x2apic,-smep,-smap,-umip"];

/// What the kernel must say when it finds no x2APIC.
///
/// The refusal D-33 chose and D-90 made a gate check: xAPIC needs page tables
/// that do not exist when the APIC is brought up, so there is no fallback —
/// only a diagnosis.
const NO_X2APIC_MARKER: &str = "no x2apic mode";

/// The forced configuration-port refusal in the degraded image.
const NO_PCI_MARKER: &str = "pci: the configuration ports are already claimed; no bus";

/// The hardware-free namespace witness targeted by the PW-207 mutation.
pub const DEGRADED_FILESYSTEM_MARKER: &str = "tmpfs:";

/// Both device names must resolve through devfs without a bus.
const DEGRADED_STORAGE_MARKER: &str = "devfs: storage is";
const DEGRADED_CONSOLE_MARKER: &str = "devfs: console is";

/// Optional hardening is absent, while mandatory WP and NX remain active.
const DEGRADED_PROTECTIONS_MARKER: &str = "cpu: protections wp nx";

/// Without a clock, the supported diagnostic boot never opens ring three.
pub const DEGRADED_USER_MARKER: &str = "user: no program: no calibrated clock; ring 3 is closed";

/// Boot with two independent platform prerequisites absent and require both.
///
/// The negative half of D-32 and D-33: every other boot in the gate runs on
/// `-cpu max` with live PCI configuration, so nothing exercises what the
/// kernel does when either is missing. A precondition that is only ever met is
/// indistinguishable from one that is never checked (D-90, D-198).
///
/// The bar is deliberately not "it works": the kernel reports both refusals,
/// exercises the namespaces which need neither facility, and still reaches
/// its steady state rather than triple-faulting on a missing register.
pub fn run_degraded_check(paths: &Paths, image: &Path, settle: Duration) -> Result<()> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;

    println!("[xtask] booting a kernel with no PCI configuration or x2apic");

    let mut child = Command::new(&installation.emulator)
        .args(MACHINE)
        .args(DEGRADED_CPU)
        .args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    let stdout = child.stdout.take().context("taking QEMU's stdout pipe")?;
    let settled = Arc::new(AtomicBool::new(false));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let required: Vec<_> = [
        NO_X2APIC_MARKER,
        NO_PCI_MARKER,
        DEGRADED_FILESYSTEM_MARKER,
        DEGRADED_STORAGE_MARKER,
        DEGRADED_CONSOLE_MARKER,
        DEGRADED_PROTECTIONS_MARKER,
        DEGRADED_USER_MARKER,
    ]
    .into_iter()
    .map(|marker| (marker, Arc::new(AtomicBool::new(false))))
    .collect();
    let forwarder = watch_serial(
        stdout,
        Arc::clone(&settled),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        required.clone(),
    );

    let outcome = watch(
        &mut child,
        &panicked,
        &serial_failed,
        BOOT_TIMEOUT,
        Some(&settled),
    )
    .and_then(|()| watch(&mut child, &panicked, &serial_failed, settle, None));

    let stop_outcome = stop_qemu(&mut child);
    let serial_outcome = finish_boot_serial(forwarder);

    stop_outcome?;
    serial_outcome?;
    outcome?;
    for (marker, seen) in required {
        if !seen.load(Ordering::Acquire) {
            bail!("the degraded kernel settled without reporting {marker:?}");
        }
    }
    println!(
        "[xtask] degraded kernel keeps optional protection fallback diagnostic-only and settles"
    );
    Ok(())
}

/// Watch the serial flags for `window`, failing on a panic or an early exit.
///
/// With `until`, the window is a deadline that flag must beat; without one,
/// the whole window is watched.
fn watch(
    child: &mut Child,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
    window: Duration,
    until: Option<&AtomicBool>,
) -> Result<()> {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        if panicked.load(Ordering::Acquire) {
            bail!("the shipped kernel panicked — see the log above");
        }
        if let Some(status) = child.try_wait().context("waiting for QEMU")? {
            bail!("the shipped kernel stopped early ({status})");
        }
        if serial_failed.load(Ordering::Acquire) {
            bail!("the shipped kernel's serial reader stopped");
        }
        if until.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    match until {
        Some(_) => bail!("the shipped kernel never reached its steady state ({SETTLED_MARKER:?})"),
        None => Ok(()),
    }
}

/// Forward every serial line, flagging the markers the boot check requires.
fn watch_serial(
    stdout: ChildStdout,
    settled: Arc<AtomicBool>,
    panicked: Arc<AtomicBool>,
    serial_failed: Arc<AtomicBool>,
    required: Vec<(&'static str, Arc<AtomicBool>)>,
) -> JoinHandle<Result<()>> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = match reader.read_until(b'\n', &mut line) {
                Ok(read) => read,
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading shipped QEMU serial output");
                }
            };
            if read == 0 {
                return Ok(());
            }
            let text = String::from_utf8_lossy(&line);
            if text.ends_with('\n') {
                print!("{text}");
            } else {
                println!("{text}");
            }
            let _ = std::io::stdout().flush();
            if text.contains(SETTLED_MARKER) {
                settled.store(true, Ordering::Release);
            }
            for (marker, seen) in &required {
                if text.contains(marker) {
                    seen.store(true, Ordering::Release);
                }
            }
            if text.contains(PANIC_MARKER) {
                panicked.store(true, Ordering::Release);
            }
        }
    })
}

fn finish_boot_serial(forwarder: JoinHandle<Result<()>>) -> Result<()> {
    forwarder
        .join()
        .map_err(|_| anyhow!("the shipped-kernel serial reader panicked"))?
}
#[cfg(test)]
mod tests {
    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct QemuFixture {
        root: PathBuf,
        template: PathBuf,
    }

    impl QemuFixture {
        fn create() -> Self {
            let root = std::env::temp_dir().join(format!(
                "wizard qemu,fixture-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).expect("QEMU fixture directory");
            let template = root.join("firmware template,pristine.fd");
            std::fs::write(&template, b"pristine firmware variables")
                .expect("QEMU variable-store template");
            Self { root, template }
        }
    }

    impl Drop for QemuFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn the_machine_type_is_a_versioned_q35_not_an_alias() {
        let Some(version) = MACHINE_TYPE.strip_prefix("pc-q35-") else {
            panic!("the QEMU machine is not a q35 model");
        };
        let Some((major, minor)) = version.split_once('.') else {
            panic!("the QEMU machine has no explicit version");
        };
        assert!(
            !major.is_empty()
                && !minor.is_empty()
                && major.bytes().all(|byte| byte.is_ascii_digit())
                && minor.bytes().all(|byte| byte.is_ascii_digit())
        );
    }

    #[test]
    fn exe_name_carries_the_platform_suffix() {
        let name = exe_name("qemu-system-x86_64");
        assert!(name.ends_with(std::env::consts::EXE_SUFFIX));
        assert!(name.starts_with("qemu-system-x86_64"));
    }

    #[test]
    fn which_in_finds_a_binary_that_is_there() {
        let this = std::env::current_exe().unwrap();
        let dir = this.parent().unwrap().to_path_buf();
        let name = this.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(which_in([dir], &name), Some(this));
    }

    #[test]
    fn which_in_reports_a_binary_that_is_not() {
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        assert_eq!(which_in([dir], "definitely-not-installed-xyz"), None);
    }

    #[test]
    fn drive_paths_escape_commas_without_quoting_spaces() {
        assert_eq!(
            drive_file("format=raw", Path::new("a space/disk,name.img")),
            [
                OsString::from("-drive"),
                OsString::from("format=raw,file=a space/disk,,name.img")
            ]
        );
    }

    #[test]
    fn shared_media_writes_use_a_private_snapshot() {
        assert_eq!(
            snapshot_drive_file("format=raw", Path::new("disk.img")),
            [
                OsString::from("-drive"),
                OsString::from("format=raw,snapshot=on,file=disk.img")
            ]
        );
    }

    #[test]
    fn simultaneous_control_sets_reserve_four_distinct_ports() {
        let first = ControlSockets::bind().expect("first control set");
        let second = ControlSockets::bind().expect("second control set");
        let addresses = [
            first.qmp.local_addr().expect("first QMP address"),
            first.monitor.local_addr().expect("first monitor address"),
            second.qmp.local_addr().expect("second QMP address"),
            second.monitor.local_addr().expect("second monitor address"),
        ];
        let mut unique = addresses.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), addresses.len());
        for address in addresses {
            assert!(
                TcpListener::bind(address).is_err(),
                "reserved control port {address} could be stolen"
            );
        }
    }

    #[test]
    fn the_monitor_dials_the_reserved_listener() {
        let controls = ControlSockets::bind().expect("control set");
        let address = controls.monitor.local_addr().expect("monitor address");
        assert_eq!(
            control_endpoint(&controls.monitor, MONITOR_CLIENT_SUFFIX).unwrap(),
            format!("tcp:{address},server=off")
        );
    }

    #[test]
    fn the_debug_endpoint_is_allocated_and_loopback_only() {
        let report = "#chr072: filename=gdb\ngdb: filename=disconnected:tcp:127.0.0.1:56552,server=on\n(qemu)";
        assert_eq!(parse_debug_endpoint(report).unwrap(), "127.0.0.1:56552");

        for refused in [
            "serial0: filename=null",
            "gdb: filename=disconnected:tcp:0.0.0.0:56552,server=on",
            "gdb: filename=disconnected:tcp:127.0.0.1:0,server=on",
            "gdb: filename=disconnected:tcp:127.0.0.1:not-a-port,server=on",
        ] {
            assert!(parse_debug_endpoint(refused).is_err(), "accepted {refused}");
        }
    }

    #[test]
    fn simultaneous_runs_get_distinct_writable_firmware() {
        let fixture = QemuFixture::create();
        let target = Arc::new(fixture.root.join("target"));
        let template = Arc::new(fixture.template.clone());
        let barrier = Arc::new(std::sync::Barrier::new(3));

        let first = {
            let target = Arc::clone(&target);
            let template = Arc::clone(&template);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                RunVars::create(&target, &template).expect("first private variable store")
            })
        };
        let second = {
            let target = Arc::clone(&target);
            let template = Arc::clone(&template);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                RunVars::create(&target, &template).expect("second private variable store")
            })
        };
        barrier.wait();
        let first = first.join().expect("first firmware creator");
        let second = second.join().expect("second firmware creator");

        assert_ne!(first.path(), second.path());
        assert_eq!(
            std::fs::read(first.path()).unwrap(),
            b"pristine firmware variables"
        );
        assert_eq!(
            std::fs::read(second.path()).unwrap(),
            b"pristine firmware variables"
        );
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();
        drop(first);
        drop(second);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
    }

    #[test]
    fn storage_construction_is_serialized_across_runners() {
        let fixture = QemuFixture::create();
        let lock_path = fixture.root.join("storage build,lock");
        let held = acquire_storage_lock(&lock_path).expect("first storage builder");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            started_tx.send(()).expect("announce contender");
            let _lock = acquire_storage_lock(&lock_path).expect("second storage builder");
            acquired_tx.send(()).expect("announce acquisition");
        });

        started_rx.recv().expect("contender started");
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
        drop(held);
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("contender acquired released lock");
        contender.join().expect("storage contender");
    }

    #[test]
    fn a_cancelled_control_wait_returns_without_a_connection() {
        let listener = control_listener("cancelled test").unwrap();
        let cancelled = AtomicBool::new(true);
        let error = accept_stream_before(
            &listener,
            Instant::now() + Duration::from_millis(1),
            Some(&cancelled),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn a_same_sized_namespace_with_different_bytes_is_not_reused() {
        assert!(namespace_is_exact(b"wizard", b"wizard"));
        assert!(!namespace_is_exact(b"wizard", b"wIzard"));
        assert!(!namespace_is_exact(b"wizard", b"wizard!"));
    }

    fn passing_verdict() -> SelftestVerdict {
        SelftestVerdict {
            status: VerdictStatus::Pass,
            passed: EXPECTED_SELFTEST_CHECKS,
            failed: 0,
            expected: EXPECTED_SELFTEST_CHECKS,
        }
    }

    /// A failing check's name is read off the line the kernel prints.
    ///
    /// Including the carriage return, which is the whole reason this test
    /// exists: without the trim the parser matches nothing and every mutation
    /// reports a survivor on a boot that went red correctly.
    #[test]
    fn a_failing_check_is_named_even_with_the_carriage_return() {
        assert_eq!(
            parse_failed_check("[error] selftest: a bound space is not reclaimed FAILED\r"),
            Some("a bound space is not reclaimed")
        );
        assert_eq!(
            parse_failed_check("[error] selftest: a thing FAILED"),
            Some("a thing")
        );
        // Not failures: the verdict line, a passing check, and unrelated text.
        assert_eq!(
            parse_failed_check("[error] selftest: verdict v1 FAIL passed=1 failed=0 expected=1"),
            None
        );
        assert_eq!(parse_failed_check("[info ] selftest: a thing"), None);
        assert_eq!(parse_failed_check("[info ] keyboard: up"), None);
    }

    #[test]
    fn versioned_verdict_is_parsed_from_a_log_record() {
        let line = format!(
            "[info ] selftest: verdict v1 PASS passed={} failed=0 expected={}\r\n",
            EXPECTED_SELFTEST_CHECKS, EXPECTED_SELFTEST_CHECKS
        );
        assert_eq!(
            parse_selftest_verdict(&line).unwrap().unwrap(),
            passing_verdict()
        );
    }

    fn baseline_line() -> &'static str {
        "[info ] selftest: baseline v1 boot_ns=11 frames_free=12 heap_slab=13 heap_large=14 heap_live=15 handles_committed=16 handles_live=17 trap_ticks=18 storage_bytes=4096 storage_ticks=19 counter_hz=20\r\n"
    }

    #[test]
    fn baseline_record_is_versioned_ordered_and_unique() {
        assert_eq!(
            parse_performance_sample(baseline_line()).unwrap().unwrap(),
            PerformanceSample {
                boot_ns: 11,
                frames_free: 12,
                heap_slab: 13,
                heap_large: 14,
                heap_live: 15,
                handles_committed: 16,
                handles_live: 17,
                trap_ticks: 18,
                storage_bytes: 4096,
                storage_ticks: 19,
                counter_hz: 20,
            }
        );
        for line in [
            baseline_line().replace("v1", "v2"),
            baseline_line().replace("boot_ns=11 frames_free=12", "frames_free=12 boot_ns=11"),
            baseline_line().replace(" counter_hz=20", ""),
            format!("{} trailing=true", baseline_line().trim_end()),
        ] {
            assert!(parse_performance_sample(&line).unwrap().is_err(), "{line}");
        }

        let mut observation = SerialObservation::default();
        observe_selftest_line(&mut observation, baseline_line()).unwrap();
        assert!(observe_selftest_line(&mut observation, baseline_line()).is_err());
    }

    #[test]
    fn a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript() {
        let failed = "[error] selftest: an earlier ownership check FAILED\r";
        let line = "[error] KERNEL PANIC on thread 'boot': panicked at source\r";
        let continuation = "exact cause\r";
        let mut observation = SerialObservation::default();
        observe_selftest_line(&mut observation, failed).unwrap();
        observe_selftest_line(&mut observation, line).unwrap();
        observe_selftest_line(&mut observation, continuation).unwrap();
        assert_eq!(
            correlate_selftest(None, observation).unwrap(),
            SelftestOutcome::Panicked {
                transcript: format!("{}\n{}", line.trim_end(), continuation.trim_end()),
                failed: vec!["an earlier ownership check".to_string()],
            }
        );
    }

    #[test]
    fn a_failed_verdict_must_name_every_failed_check() {
        let observation = SerialObservation {
            verdict: Some(SelftestVerdict {
                status: VerdictStatus::Fail,
                passed: EXPECTED_SELFTEST_CHECKS - 1,
                failed: 1,
                expected: EXPECTED_SELFTEST_CHECKS,
            }),
            baseline: None,
            failed: Vec::new(),
            panic: None,
        };
        let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
        assert!(error.to_string().contains("counted 1 failures but named 0"));
    }

    #[test]
    fn malformed_or_unknown_verdicts_are_rejected() {
        for line in [
            "selftest: verdict v2 PASS passed=76 failed=0 expected=76",
            "selftest: verdict v1 MAYBE passed=76 failed=0 expected=76",
            "selftest: verdict v1 PASS passed=many failed=0 expected=76",
            "selftest: verdict v1 PASS passed=76 failed=0 expected=76 trailing",
        ] {
            assert!(parse_selftest_verdict(line).unwrap().is_err(), "{line}");
        }
    }

    #[test]
    fn a_second_final_verdict_is_rejected() {
        let line = format!(
            "selftest: verdict v1 PASS passed={} failed=0 expected={}",
            EXPECTED_SELFTEST_CHECKS, EXPECTED_SELFTEST_CHECKS
        );
        let mut observation = SerialObservation::default();
        observe_selftest_line(&mut observation, &line).unwrap();
        assert!(observe_selftest_line(&mut observation, &line).is_err());
    }

    #[test]
    fn a_valid_serial_pass_must_match_debug_exit() {
        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(passing_verdict()),
            baseline: None,
            panic: None,
        };
        assert!(correlate_selftest(Some(SELFTEST_PASSED), observation).is_ok());

        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(passing_verdict()),
            baseline: None,
            panic: None,
        };
        let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
        assert!(error.to_string().contains("debug-exit"));
    }

    #[test]
    fn zero_or_skipped_checks_cannot_pass() {
        for passed in [0, EXPECTED_SELFTEST_CHECKS - 1] {
            let observation = SerialObservation {
                failed: Vec::new(),
                verdict: Some(SelftestVerdict {
                    status: VerdictStatus::Pass,
                    passed,
                    failed: 0,
                    expected: EXPECTED_SELFTEST_CHECKS,
                }),
                baseline: None,
                panic: None,
            };
            let error = correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap_err();
            assert!(error.to_string().contains("counted"));
        }
    }

    #[test]
    fn harness_and_kernel_must_agree_on_the_expected_count() {
        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(SelftestVerdict {
                expected: EXPECTED_SELFTEST_CHECKS - 1,
                passed: EXPECTED_SELFTEST_CHECKS - 1,
                ..passing_verdict()
            }),
            baseline: None,
            panic: None,
        };
        let error = correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap_err();
        assert!(error.to_string().contains("harness requires"));
    }

    #[test]
    fn serial_status_must_agree_with_its_counts() {
        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(SelftestVerdict {
                status: VerdictStatus::Fail,
                ..passing_verdict()
            }),
            baseline: None,
            panic: None,
        };
        let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
        assert!(error.to_string().contains("contradicts its counts"));
    }

    #[test]
    fn a_debug_exit_without_a_serial_verdict_is_rejected() {
        let error =
            correlate_selftest(Some(SELFTEST_PASSED), SerialObservation::default()).unwrap_err();
        assert!(error.to_string().contains("without a serial"));
    }

    #[test]
    fn irq_statistics_sum_matching_controllers() {
        let report = "IRQ statistics for i8259:\r\n 0: 12\r\n 1: 2\r\n\
                      IRQ statistics for ioapic:\r\n IRQ1: 3\r\n 12: 99\r\n(qemu)";
        assert_eq!(irq_count(report, 1), Some(5));
    }

    #[test]
    fn non_irq_monitor_output_is_not_a_zero_count() {
        assert_eq!(irq_count("unknown command: info irq\r\n(qemu)", 1), None);
    }

    /// Every constant this module shares with the kernel is read back out of
    /// the kernel's own source.
    ///
    /// Four contracts are written twice — the check count, the verdict
    /// version, the two serial markers and the panic marker — and nothing
    /// checked the copies agreed. A count drift is invisible to `cargo test`
    /// and surfaces only after a sixty-second boot; a marker drift makes the
    /// harness wait for a line that will never come. This is the shape
    /// `paths.rs`'s `default-members` test exists for, applied to the other
    /// four lists (D-105).
    /// The exit codes the harness waits for are the kernel's, doubled.
    ///
    /// `SELFTEST_PASSED`/`SELFTEST_FAILED` are a twin of `debug_exit.rs`'s
    /// `PASSED`/`FAILED`, and were "kept in step" by a comment saying so.
    /// Nothing compared them, and the serial-contract test beside this one
    /// reads `selftest.rs`, `main.rs` and `panic.rs` — not the file that
    /// writes the port.
    ///
    /// What that bought: change `FAILED` to `0x12` and `cargo xtask ci` stays
    /// green, because every boot in the gate is a *passing* one. But every
    /// mutation reports "inconclusive", because `mutate` can no longer tell a
    /// red verdict from a broken build — one character disables the tool that
    /// keeps D-28 true, and the gate says nothing (D-135).
    ///
    /// Derived, not restated: the harness value is computed from the kernel's
    /// by QEMU's own `(value << 1) | 1`, so this cannot drift by agreeing with
    /// itself.
    #[test]
    fn the_exit_codes_are_the_kernels_doubled() {
        /// What QEMU's `isa-debug-exit` does to the value written.
        fn qemu_exit_code(written: i32) -> i32 {
            (written << 1) | 1
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let debug_exit = std::fs::read_to_string(root.join("kernel/src/arch/x86_64/debug_exit.rs"))
            .expect("debug_exit.rs");

        let value = |name: &str| -> i32 {
            let marked = format!("const {name}: u32 = ");
            let rest = debug_exit
                .split_once(&marked)
                .unwrap_or_else(|| panic!("no `{name}` in debug_exit.rs"))
                .1;
            let literal = rest
                .split_once(';')
                .expect("a terminated constant")
                .0
                .trim()
                .trim_start_matches("0x");
            i32::from_str_radix(literal, 16).expect("a hex literal")
        };

        assert_eq!(
            qemu_exit_code(value("PASSED")),
            SELFTEST_PASSED,
            "the kernel's PASSED does not arrive as the code the harness waits for"
        );
        assert_eq!(
            qemu_exit_code(value("FAILED")),
            SELFTEST_FAILED,
            "the kernel's FAILED does not arrive as the code the harness waits for"
        );
        // And they are distinguishable, which is the whole point of two.
        assert_ne!(SELFTEST_PASSED, SELFTEST_FAILED);
    }

    #[test]
    fn the_serial_contract_matches_the_kernel_source() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
            .join("kernel")
            .join("src");
        let selftest = std::fs::read_to_string(root.join("selftest.rs")).expect("selftest.rs");
        let main = std::fs::read_to_string(root.join("main.rs")).expect("main.rs");
        let devfs = std::fs::read_to_string(root.join("devfs.rs")).expect("devfs.rs");
        let panic = std::fs::read_to_string(root.join("panic.rs")).expect("panic.rs");

        let constant = |source: &str, name: &str| -> String {
            source
                .lines()
                .find_map(|line| line.trim().strip_prefix(&format!("const {name}: usize = ")))
                .map(|value| value.trim_end_matches(';').to_string())
                .unwrap_or_else(|| panic!("no `{name}` in the kernel source"))
        };

        assert_eq!(
            constant(&selftest, "EXPECTED_CHECKS"),
            EXPECTED_SELFTEST_CHECKS.to_string(),
            "the check count drifted"
        );
        assert_eq!(
            format!("v{}", constant(&selftest, "VERDICT_VERSION")),
            SELFTEST_VERDICT_VERSION,
            "the verdict version drifted"
        );
        assert!(
            selftest.contains(SELFTEST_VERDICT_PREFIX.trim_end()),
            "the kernel does not print {SELFTEST_VERDICT_PREFIX:?}"
        );
        assert!(
            selftest.contains(STORAGE_DELAY_MARKER),
            "the kernel does not print {STORAGE_DELAY_MARKER:?}"
        );
        assert!(
            selftest.contains(READY_MARKER),
            "the kernel does not print {READY_MARKER:?}"
        );
        // Both markers, or the second injection round waits for a line that
        // never comes and the translated keystroke is red for the harness's
        // reason rather than the kernel's (D-179).
        assert!(
            selftest.contains(TRANSLATED_MARKER),
            "the kernel does not print {TRANSLATED_MARKER:?}"
        );
        assert!(
            main.contains(SETTLED_MARKER),
            "the kernel does not print {SETTLED_MARKER:?}"
        );
        assert!(
            main.contains(NO_PCI_MARKER),
            "the kernel does not print {NO_PCI_MARKER:?}"
        );
        assert!(
            main.contains(DEGRADED_FILESYSTEM_MARKER),
            "the kernel does not print {DEGRADED_FILESYSTEM_MARKER:?}"
        );
        assert!(
            main.contains("devfs: {} is {}")
                && devfs.contains("storage")
                && devfs.contains("console"),
            "the devfs degraded markers no longer compose from the kernel sources"
        );
        assert!(
            panic.contains(PANIC_MARKER),
            "the kernel does not print {PANIC_MARKER:?}"
        );
    }

    /// The signature the namespace carries is the one the kernel looks for.
    ///
    /// Two transcriptions of one table again (D-158): `xtask` writes the
    /// backing file and `selftest.rs` checks what comes back. They agree today
    /// and nothing compared them, which is how a check that proves a disk read
    /// happened becomes a check that proves two constants were edited
    /// together — or, worse, silently stops proving anything when only one is
    /// (D-169).
    #[test]
    fn the_namespaces_signature_is_the_one_the_kernel_looks_for() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let selftest =
            std::fs::read_to_string(root.join("kernel/src/selftest.rs")).expect("selftest.rs");

        let kernel = selftest
            .lines()
            .find_map(|line| line.trim().strip_prefix("0x5749_5A44_0000_0000 | "))
            .expect("no disk signature in selftest.rs");
        assert_eq!(
            kernel, "lba",
            "the kernel's signature is not the base value ored with the block"
        );

        // And the base itself, from this file's own function rather than from
        // a literal repeated here.
        assert_eq!(super::nvme_signature(0), 0x5749_5A44_0000_0000);
        for lba in [0u64, 1, 3, 7, super::NVME_SIGNED_BLOCKS - 1] {
            assert_eq!(
                super::nvme_signature(lba),
                0x5749_5A44_0000_0000 | lba,
                "block {lba}"
            );
        }
        // Every signed block differs from every other, which is the property
        // that makes a read of block three distinguishable from a read of
        // block zero.
        for lba in 0..super::NVME_SIGNED_BLOCKS {
            for earlier in 0..lba {
                assert_ne!(super::nvme_signature(lba), super::nvme_signature(earlier));
            }
        }
    }

    /// Queue addresses have a cleanup owner before the controller sees them.
    #[test]
    fn nvme_cleanup_ownership_precedes_the_enable_write() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let source =
            std::fs::read_to_string(root.join("kernel/src/nvme.rs")).expect("kernel nvme source");
        let owner = source
            .find("let mut controller = Controller {")
            .expect("no cleanup-owning controller construction");
        let enable = source
            .find("register::CC, cc")
            .expect("no controller enable write");

        assert!(
            owner < enable,
            "the controller can see host queue addresses before cleanup owns them"
        );
    }

    /// The corpus the kernel looks for is the corpus `xtask` writes.
    ///
    /// Three transcriptions meet here: the names and contents live in
    /// `image/corpus.rs`, the kernel's checks name them again, and the volume's
    /// base sector is stated in both `qemu.rs` and `kernel/src/fs.rs`. Two
    /// copies of one table is what D-158 exists about, and a check that reads
    /// a file the host never wrote would fail loudly — while a check that read
    /// the *wrong* file would quietly pass on whatever happened to be there.
    #[test]
    fn the_corpus_the_kernel_looks_for_is_the_one_that_is_written() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let selftest =
            std::fs::read_to_string(root.join("kernel/src/selftest.rs")).expect("selftest.rs");
        let fs = std::fs::read_to_string(root.join("kernel/src/fs.rs")).expect("fs.rs");
        use crate::image::corpus;

        // The base sector, stated twice.
        let kernel_base = fs
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("pub(crate) const VOLUME_SECTOR: u64 = ")
            })
            .map(|value| value.trim_end_matches(';').to_string())
            .expect("no VOLUME_SECTOR in fs.rs");
        assert_eq!(kernel_base, super::NVME_VOLUME_SECTOR.to_string());

        // Every name and every content, as the kernel spells them.
        for (constant, expected) in [
            ("SMALL_NAME", corpus::SMALL_NAME.to_string()),
            ("LARGE_NAME", corpus::LARGE_NAME.to_string()),
            ("DIRECTORY_NAME", corpus::DIRECTORY_NAME.to_string()),
            ("NESTED_NAME", corpus::NESTED_NAME.to_string()),
        ] {
            let declared = selftest
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix(&format!("const {constant}: &[u8] = b\""))
                })
                .and_then(|rest| rest.split_once('\"').map(|(value, _)| value.to_string()))
                .unwrap_or_else(|| panic!("no {constant} in selftest.rs"));
            assert_eq!(declared, expected, "{constant} drifted");
        }

        // The large file's size, and the function that fills it.
        assert!(
            selftest.contains(&format!(
                "const LARGE_BYTES: usize = {} * 1024;",
                corpus::LARGE_BYTES / 1024
            )),
            "LARGE_BYTES drifted from the corpus"
        );
        assert!(
            selftest.contains("((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8"),
            "the kernel fills the large file differently from xtask"
        );
        // And that function really is the one `corpus` uses, rather than a
        // literal repeated in this test.
        for index in [0usize, 1, 511, 4096, corpus::LARGE_BYTES - 1] {
            assert_eq!(
                corpus::large_byte(index),
                ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
            );
        }
    }

    fn cpp_raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
        if bytes.get(start..start + 2) != Some(b"R\"") {
            return None;
        }
        let identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        let boundary = |at: usize| at == 0 || !identifier(bytes[at - 1]);
        let has_prefix = boundary(start)
            || (start >= 1
                && matches!(bytes[start - 1], b'u' | b'U' | b'L')
                && boundary(start - 1))
            || (start >= 2 && &bytes[start - 2..start] == b"u8" && boundary(start - 2));
        if !has_prefix {
            return None;
        }
        let delimiter_at = start + 2;
        let open = bytes[delimiter_at..]
            .iter()
            .position(|byte| *byte == b'(')
            .map(|offset| delimiter_at + offset)
            .expect("unterminated C++ raw-string delimiter");
        let delimiter = &bytes[delimiter_at..open];
        assert!(delimiter.len() <= 16, "oversized C++ raw-string delimiter");
        assert!(
            delimiter.iter().all(|byte| {
                !byte.is_ascii_whitespace() && !matches!(*byte, b'(' | b')' | b'\\')
            }),
            "invalid C++ raw-string delimiter"
        );
        let terminator_bytes = 2 + delimiter.len();
        let close = bytes[open + 1..]
            .windows(terminator_bytes)
            .position(|window| {
                window[0] == b')'
                    && &window[1..1 + delimiter.len()] == delimiter
                    && window[1 + delimiter.len()] == b'"'
            })
            .map(|offset| open + 1 + offset + terminator_bytes)
            .expect("unterminated C++ raw string literal");
        Some(close)
    }

    fn cpp_without_comments(source: &str) -> (String, Vec<String>) {
        for trigraph in [
            "??=", "??/", "??'", "??(", "??)", "??!", "??<", "??>", "??-",
        ] {
            assert!(
                !source.contains(trigraph),
                "C/C++ trigraph {trigraph} is not admitted"
            );
        }
        for (line_number, line) in source.lines().enumerate() {
            assert!(
                !line.trim_end().ends_with('\\'),
                "C/C++ line continuation on line {}",
                line_number + 1
            );
        }
        let bytes = source.as_bytes();
        let mut active = bytes.to_vec();
        let mut line_comments = Vec::new();
        let mut index = 0;

        while index < bytes.len() {
            if let Some(end) = cpp_raw_string_end(bytes, index) {
                index = end;
                continue;
            }
            match bytes[index] {
                quote @ (b'\'' | b'"') => {
                    index += 1;
                    let mut closed = false;
                    while index < bytes.len() {
                        if bytes[index] == b'\\' {
                            index = (index + 2).min(bytes.len());
                        } else if bytes[index] == quote {
                            index += 1;
                            closed = true;
                            break;
                        } else {
                            index += 1;
                        }
                    }
                    assert!(closed, "unterminated C++ literal");
                }
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    let end = bytes[index + 2..]
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or(bytes.len(), |offset| index + 2 + offset);
                    line_comments.push(
                        String::from_utf8_lossy(&bytes[index + 2..end])
                            .trim()
                            .to_string(),
                    );
                    active[index..end].fill(b' ');
                    index = end;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    let start = index;
                    index += 2;
                    let mut closed = false;
                    while index + 1 < bytes.len() {
                        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
                            index += 2;
                            closed = true;
                            break;
                        }
                        index += 1;
                    }
                    assert!(closed, "unterminated C++ block comment");
                    for byte in &mut active[start..index] {
                        if *byte != b'\n' && *byte != b'\r' {
                            *byte = b' ';
                        }
                    }
                }
                _ => index += 1,
            }
        }

        (
            String::from_utf8(active).expect("comment masking kept UTF-8"),
            line_comments,
        )
    }

    fn cpp_structure(active: &str) -> String {
        let bytes = active.as_bytes();
        let mut structure = bytes.to_vec();
        let mut index = 0;
        while index < bytes.len() {
            if let Some(end) = cpp_raw_string_end(bytes, index) {
                for byte in &mut structure[index..end] {
                    if *byte != b'\n' && *byte != b'\r' {
                        *byte = b' ';
                    }
                }
                index = end;
                continue;
            }
            let quote = bytes[index];
            if quote != b'\'' && quote != b'"' {
                index += 1;
                continue;
            }
            let start = index;
            index += 1;
            let mut closed = false;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else if bytes[index] == quote {
                    index += 1;
                    closed = true;
                    break;
                } else {
                    index += 1;
                }
            }
            assert!(closed, "unterminated C++ literal");
            for byte in &mut structure[start..index] {
                if *byte != b'\n' && *byte != b'\r' {
                    *byte = b' ';
                }
            }
        }
        String::from_utf8(structure).expect("literal masking kept UTF-8")
    }

    fn rust_without_comments(source: &str) -> String {
        let bytes = source.as_bytes();
        let mut active = bytes.to_vec();
        let mut index = 0usize;

        while index < bytes.len() {
            if bytes[index] == b'r' {
                let mut quote = index + 1;
                while bytes.get(quote) == Some(&b'#') {
                    quote += 1;
                }
                if bytes.get(quote) == Some(&b'"') {
                    let hashes = quote - index - 1;
                    let mut end = quote + 1;
                    let mut closed = None;
                    while end < bytes.len() {
                        if bytes[end] == b'"'
                            && bytes.get(end + 1..end + 1 + hashes)
                                == Some(&bytes[index + 1..quote])
                        {
                            closed = Some(end + 1 + hashes);
                            break;
                        }
                        end += 1;
                    }
                    let end = closed.expect("unterminated Rust raw string literal");
                    for byte in &mut active[index..end] {
                        if *byte != b'\n' && *byte != b'\r' {
                            *byte = b' ';
                        }
                    }
                    index = end;
                    continue;
                }
            }
            match bytes[index] {
                b'"' => {
                    let start = index;
                    index += 1;
                    let mut closed = false;
                    while index < bytes.len() {
                        if bytes[index] == b'\\' {
                            index = (index + 2).min(bytes.len());
                        } else if bytes[index] == b'"' {
                            index += 1;
                            closed = true;
                            break;
                        } else {
                            index += 1;
                        }
                    }
                    assert!(closed, "unterminated Rust string literal");
                    for byte in &mut active[start..index] {
                        if *byte != b'\n' && *byte != b'\r' {
                            *byte = b' ';
                        }
                    }
                }
                b'\'' => {
                    let start = index;
                    let mut cursor = index + 1;
                    let mut escaped = false;
                    let mut close = None;
                    while cursor < bytes.len() && bytes[cursor] != b'\n' {
                        if !escaped && bytes[cursor] == b'\'' {
                            close = Some(cursor + 1);
                            break;
                        }
                        escaped = !escaped && bytes[cursor] == b'\\';
                        cursor += 1;
                    }
                    let is_character = close.is_some_and(|end| {
                        let contents = &source[start + 1..end - 1];
                        contents.starts_with('\\') || contents.chars().count() == 1
                    });
                    if is_character {
                        index = close.expect("a character literal has a close");
                        active[start..index].fill(b' ');
                    } else {
                        index += 1;
                    }
                }
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    let end = bytes[index + 2..]
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or(bytes.len(), |offset| index + 2 + offset);
                    active[index..end].fill(b' ');
                    index = end;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    let start = index;
                    let mut depth = 1usize;
                    index += 2;
                    while index < bytes.len() && depth != 0 {
                        if bytes.get(index + 1) == Some(&b'*') && bytes[index] == b'/' {
                            depth += 1;
                            index += 2;
                        } else if bytes.get(index + 1) == Some(&b'/') && bytes[index] == b'*' {
                            depth -= 1;
                            index += 2;
                        } else {
                            index += 1;
                        }
                    }
                    assert_eq!(depth, 0, "unterminated Rust block comment");
                    for byte in &mut active[start..index] {
                        if *byte != b'\n' && *byte != b'\r' {
                            *byte = b' ';
                        }
                    }
                }
                _ => index += 1,
            }
        }

        String::from_utf8(active).expect("Rust comment masking kept UTF-8")
    }

    fn cpp_directive(line: &str) -> Option<(&str, &str)> {
        let directive = line.trim_start().strip_prefix('#')?.trim_start();
        let split = directive
            .bytes()
            .position(|byte| byte.is_ascii_whitespace())
            .unwrap_or(directive.len());
        Some((&directive[..split], directive[split..].trim()))
    }

    fn checked_header(source: &str) -> (String, std::collections::BTreeMap<String, String>) {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Phase {
            BeforeGuard,
            GuardDefinition,
            Includes,
            LinkageOpen,
            Body,
            LinkageClose,
            GuardClose,
            Done,
        }

        let (active, _) = cpp_without_comments(source);
        let structure = cpp_structure(&active);
        let mut macros = std::collections::BTreeMap::new();
        let mut includes = Vec::new();
        let mut linkage_body = String::new();
        let mut phase = Phase::BeforeGuard;

        for (line_number, (line, structural_line)) in
            active.lines().zip(structure.lines()).enumerate()
        {
            let structural_line = structural_line.trim();
            assert!(
                !structural_line.starts_with("%:") && !structural_line.starts_with("??="),
                "alternate preprocessor directive on line {}",
                line_number + 1
            );
            assert!(
                !structural_line
                    .split(|character: char| {
                        !character.is_ascii_alphanumeric() && character != '_'
                    })
                    .any(|word| matches!(word, "_Pragma" | "__pragma")),
                "pragma operator on line {}",
                line_number + 1
            );
            if cpp_directive(structural_line).is_some() {
                let (kind, rest) = cpp_directive(line).expect("structural directive has source");
                assert!(
                    !line.trim_end().ends_with('\\'),
                    "continued header directive on line {}",
                    line_number + 1
                );
                match kind {
                    "ifndef" => {
                        assert_eq!(phase, Phase::BeforeGuard, "misplaced include guard");
                        assert_eq!(rest, "WIZARD_ABI_H", "unexpected header guard");
                        phase = Phase::GuardDefinition;
                    }
                    "ifdef" => {
                        assert_eq!(rest, "__cplusplus", "unexpected header conditional");
                        phase = match phase {
                            Phase::Includes => {
                                assert_eq!(
                                    includes,
                                    ["<stddef.h>", "<stdint.h>"],
                                    "the fixed header includes changed"
                                );
                                linkage_body.clear();
                                Phase::LinkageOpen
                            }
                            Phase::Body => {
                                linkage_body.clear();
                                Phase::LinkageClose
                            }
                            _ => panic!("misplaced __cplusplus guard"),
                        };
                    }
                    "endif" => {
                        assert!(rest.is_empty(), "unexpected tokens after #endif");
                        phase = match phase {
                            Phase::LinkageOpen => {
                                assert_eq!(
                                    linkage_body, "extern\"C\"{",
                                    "the first __cplusplus guard is not the linkage opener"
                                );
                                Phase::Body
                            }
                            Phase::LinkageClose => {
                                assert_eq!(
                                    linkage_body, "}",
                                    "the second __cplusplus guard is not the linkage closer"
                                );
                                Phase::GuardClose
                            }
                            Phase::GuardClose => Phase::Done,
                            _ => panic!("#endif without the expected opener"),
                        };
                    }
                    "include" => {
                        assert_eq!(phase, Phase::Includes, "misplaced header include");
                        assert!(
                            matches!(rest, "<stddef.h>" | "<stdint.h>"),
                            "unexpected header include `{rest}`"
                        );
                        assert!(!includes.contains(&rest), "duplicate include `{rest}`");
                        includes.push(rest);
                    }
                    "define" => {
                        let split = rest
                            .bytes()
                            .position(|byte| byte.is_ascii_whitespace())
                            .unwrap_or(rest.len());
                        let name = &rest[..split];
                        let value = rest[split..].trim();
                        if name == "WIZARD_ABI_H" {
                            assert_eq!(
                                phase,
                                Phase::GuardDefinition,
                                "misplaced include guard definition"
                            );
                            assert!(value.is_empty(), "the include guard has a value");
                            phase = Phase::Includes;
                            continue;
                        }
                        assert_eq!(phase, Phase::Body, "macro outside the linkage body");
                        assert!(name.starts_with("WIZ_"), "unexpected macro `{name}`");
                        assert!(
                            name.bytes().all(|byte| byte.is_ascii_uppercase()
                                || byte.is_ascii_digit()
                                || byte == b'_'),
                            "non-object macro `{name}`"
                        );
                        assert!(!value.is_empty(), "macro `{name}` has no value");
                        assert!(
                            macros.insert(name.to_string(), value.to_string()).is_none(),
                            "duplicate macro `{name}`"
                        );
                    }
                    other => panic!("unexpected header directive #{other}"),
                }
                continue;
            }

            let code = compact_cpp(line);
            match phase {
                Phase::LinkageOpen | Phase::LinkageClose => linkage_body.push_str(&code),
                Phase::Body => {}
                _ => assert!(
                    code.is_empty(),
                    "code outside the header's declared linkage body"
                ),
            }
        }

        assert_eq!(phase, Phase::Done, "incomplete header conditional scaffold");
        (active, macros)
    }

    fn matching(source: &[u8], open: usize, left: u8, right: u8) -> usize {
        let mut depth = 0usize;
        for (offset, byte) in source[open..].iter().enumerate() {
            if *byte == left {
                depth += 1;
            } else if *byte == right {
                depth = depth.checked_sub(1).expect("closing delimiter before open");
                if depth == 0 {
                    return open + offset;
                }
            }
        }
        panic!("unterminated delimiter at byte {open}");
    }

    fn cpp_bodies(active: &str) -> std::collections::BTreeMap<String, (&str, &str)> {
        let structure = cpp_structure(active);
        let bytes = structure.as_bytes();
        let outer_marker = "namespace mlibc";
        let outer_markers: Vec<_> = structure.match_indices(outer_marker).collect();
        assert_eq!(outer_markers.len(), 1, "expected one namespace mlibc");
        assert!(
            outer_markers[0].0 == 0
                || !bytes[outer_markers[0].0 - 1].is_ascii_alphanumeric()
                    && bytes[outer_markers[0].0 - 1] != b'_',
            "namespace mlibc is embedded in an identifier"
        );
        let mut outer_open = outer_markers[0].0 + outer_marker.len();
        while bytes.get(outer_open).is_some_and(u8::is_ascii_whitespace) {
            outer_open += 1;
        }
        assert_eq!(
            bytes.get(outer_open),
            Some(&b'{'),
            "namespace mlibc has unexpected qualifiers"
        );
        let outer_close = matching(bytes, outer_open, b'{', b'}');
        assert!(
            structure[outer_close + 1..].trim().is_empty(),
            "active C++ follows namespace mlibc"
        );
        let inner_marker = "namespace {";
        let inner_markers: Vec<_> = structure[outer_open + 1..outer_close]
            .match_indices(inner_marker)
            .map(|(at, marker)| (outer_open + 1 + at, marker))
            .filter(|(at, _)| {
                *at == 0 || !bytes[*at - 1].is_ascii_alphanumeric() && bytes[*at - 1] != b'_'
            })
            .collect();
        assert_eq!(inner_markers.len(), 1, "expected one anonymous namespace");
        let inner_open = inner_markers[0].0 + inner_marker.len() - 1;
        let inner_close = matching(bytes, inner_open, b'{', b'}');
        assert!(
            inner_close < outer_close,
            "anonymous namespace escapes mlibc"
        );
        let mut bodies = std::collections::BTreeMap::new();
        let mut index = 0usize;
        let mut depth = 0usize;

        while index < bytes.len() {
            if bytes[index] == b'{' {
                depth += 1;
                index += 1;
                continue;
            }
            if bytes[index] == b'}' {
                depth = depth.checked_sub(1).expect("unmatched C++ closing brace");
                index += 1;
                continue;
            }
            if !bytes[index].is_ascii_alphabetic() && bytes[index] != b'_' {
                index += 1;
                continue;
            }
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let name = &structure[start..index];
            if !name.starts_with("sys_")
                && !matches!(
                    name,
                    "call1" | "as_errno" | "read_counter" | "monotonic_fallback"
                )
            {
                continue;
            }
            let mut cursor = index;
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b'(') {
                continue;
            }
            let close_parameters = matching(bytes, cursor, b'(', b')');
            cursor = close_parameters + 1;
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b'{') {
                continue;
            }
            let (expected_depth, in_expected_namespace) = if name.starts_with("sys_") {
                (
                    1,
                    start > outer_open
                        && start < outer_close
                        && !(start > inner_open && start < inner_close),
                )
            } else {
                (2, start > inner_open && start < inner_close)
            };
            assert_eq!(
                depth, expected_depth,
                "{name} is not defined at its expected mlibc namespace depth"
            );
            assert!(
                in_expected_namespace,
                "{name} is outside its expected mlibc namespace"
            );
            let close_body = matching(bytes, cursor, b'{', b'}');
            let declaration_start = structure[..start]
                .rfind('\n')
                .map_or(0, |newline| newline + 1);
            assert!(
                bodies
                    .insert(
                        name.to_string(),
                        (
                            active[declaration_start..cursor].trim(),
                            &active[cursor + 1..close_body],
                        ),
                    )
                    .is_none(),
                "two active definitions of {name}"
            );
            index = close_body + 1;
        }
        assert_eq!(depth, 0, "unbalanced C++ braces");
        bodies
    }

    fn sysdep_annotations(
        line_comments: &[String],
    ) -> std::collections::BTreeMap<String, (String, String)> {
        let mut annotations = std::collections::BTreeMap::new();
        for comment in line_comments {
            let Some(rest) = comment.strip_prefix("sys_") else {
                continue;
            };
            let Some((name, states)) = rest.split_once(". BODY=") else {
                continue;
            };
            let (body, remainder) = states
                .split_once("; REMAINDER=")
                .unwrap_or_else(|| panic!("sys_{name} has no REMAINDER state"));
            let remainder = remainder
                .strip_suffix('.')
                .unwrap_or_else(|| panic!("sys_{name}'s state marker has no full stop"));
            assert!(
                matches!(body, "WIRED" | "COMPOSED" | "PARTIAL" | "STUB"),
                "sys_{name} has unknown BODY={body}"
            );
            assert!(
                matches!(
                    remainder,
                    "WORKS" | "COMPAT" | "DOOR" | "PARTIAL" | "MISSING"
                ),
                "sys_{name} has unknown REMAINDER={remainder}"
            );
            assert!(
                annotations
                    .insert(
                        format!("sys_{name}"),
                        (body.to_string(), remainder.to_string())
                    )
                    .is_none(),
                "sys_{name} has two state markers"
            );
        }
        annotations
    }

    fn compact_cpp(source: &str) -> String {
        source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    }

    fn assert_clock_arithmetic(source: &str) {
        let compact = compact_cpp(&cpp_structure(source));
        let start = compact
            .find("uint64_tticks=")
            .expect("clock counter-delta calculation");
        let end = compact[start..]
            .find("*secs=")
            .map(|offset| start + offset)
            .expect("clock result stores");
        assert_eq!(
            &compact[start..end],
            concat!(
                "uint64_tticks=stamp-last;",
                "unsigned__int128advanced=static_cast<unsigned__int128>(fraction)",
                "+static_cast<unsigned__int128>(ticks)*multiplier;",
                "unsigned__int128whole=advanced>>shift;",
                "uint64_tns;",
                "if(whole>UINT64_MAX){ns=UINT64_MAX;}",
                "else{uint64_tdelta=static_cast<uint64_t>(whole);",
                "ns=base>UINT64_MAX-delta?UINT64_MAX:base+delta;}"
            ),
            "the clock arithmetic no longer widens before multiplying and saturates afterwards"
        );
    }

    struct CIntegerParser<'a> {
        source: &'a [u8],
        at: usize,
    }

    impl CIntegerParser<'_> {
        fn skip_space(&mut self) {
            while self
                .source
                .get(self.at)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.at += 1;
            }
        }

        fn take(&mut self, token: &[u8]) -> bool {
            self.skip_space();
            if self.source.get(self.at..self.at + token.len()) == Some(token) {
                self.at += token.len();
                true
            } else {
                false
            }
        }

        fn identifier(&mut self) -> Option<String> {
            self.skip_space();
            let start = self.at;
            if !self
                .source
                .get(self.at)
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            {
                return None;
            }
            self.at += 1;
            while self
                .source
                .get(self.at)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                self.at += 1;
            }
            Some(
                std::str::from_utf8(&self.source[start..self.at])
                    .expect("ASCII identifier")
                    .to_string(),
            )
        }

        fn number(&mut self) -> Result<u64, String> {
            self.skip_space();
            let start = self.at;
            let (radix, digits_at) = if self.source.get(self.at..self.at + 2) == Some(b"0x")
                || self.source.get(self.at..self.at + 2) == Some(b"0X")
            {
                self.at += 2;
                (16, self.at)
            } else {
                (10, self.at)
            };
            while self.source.get(self.at).is_some_and(|byte| match radix {
                16 => byte.is_ascii_hexdigit(),
                _ => byte.is_ascii_digit(),
            }) {
                self.at += 1;
            }
            if self.at == digits_at {
                return Err(format!("expected an integer literal at byte {start}"));
            }
            let digits = std::str::from_utf8(&self.source[digits_at..self.at])
                .expect("ASCII integer literal");
            if radix == 10 && digits.len() > 1 && digits.starts_with('0') {
                return Err(format!(
                    "octal-looking integer literal `{digits}` is not admitted"
                ));
            }
            let suffix_at = self.at;
            while self
                .source
                .get(self.at)
                .is_some_and(u8::is_ascii_alphabetic)
            {
                self.at += 1;
            }
            let suffix = std::str::from_utf8(&self.source[suffix_at..self.at])
                .expect("ASCII integer suffix")
                .to_ascii_lowercase();
            if !matches!(
                suffix.as_str(),
                "" | "u" | "l" | "ul" | "lu" | "ll" | "ull" | "llu"
            ) {
                return Err(format!("unsupported integer suffix `{suffix}`"));
            }
            u64::from_str_radix(digits, radix)
                .map_err(|_| format!("integer literal `{digits}` does not fit in u64"))
        }

        fn primary(&mut self) -> Result<u64, String> {
            self.skip_space();
            if !self.take(b"(") {
                return self.number();
            }

            let inside = self.at;
            if let Some(kind) = self.identifier() {
                self.skip_space();
                if self.take(b")") {
                    if !matches!(
                        kind.as_str(),
                        "wiz_handle_t" | "wiz_rights_t" | "uintptr_t" | "uint64_t"
                    ) {
                        return Err(format!("unsupported C cast `{kind}`"));
                    }
                    return self.primary();
                }
            }

            self.at = inside;
            let value = self.shift()?;
            if !self.take(b")") {
                return Err(format!(
                    "unterminated parenthesized integer at byte {inside}"
                ));
            }
            Ok(value)
        }

        fn shift(&mut self) -> Result<u64, String> {
            let mut value = self.primary()?;
            while self.take(b"<<") {
                let shift = self.primary()?;
                if shift >= u64::from(u32::BITS) || value > (u64::from(u32::MAX) >> shift) {
                    return Err("overflowing integer shift".to_string());
                }
                value <<= shift;
            }
            Ok(value)
        }
    }

    fn parse_c_uint(expression: &str) -> Result<u64, String> {
        let mut parser = CIntegerParser {
            source: expression.as_bytes(),
            at: 0,
        };
        let value = parser.shift()?;
        parser.skip_space();
        if parser.at != parser.source.len() {
            return Err(format!(
                "unparsed C integer expression `{}`",
                &expression[parser.at..]
            ));
        }
        Ok(value)
    }

    fn parse_rust_uint(literal: &str) -> Result<u64, String> {
        let literal = literal.trim();
        let (radix, prefix) = if literal.starts_with("0x") {
            (16, 2)
        } else if literal.starts_with("0b") {
            (2, 2)
        } else if literal.starts_with("0o") {
            (8, 2)
        } else {
            (10, 0)
        };
        let mut end = prefix;
        for (offset, character) in literal[prefix..].char_indices() {
            let digit = character == '_'
                || match radix {
                    16 => character.is_ascii_hexdigit(),
                    10 => character.is_ascii_digit(),
                    8 => matches!(character, '0'..='7'),
                    2 => matches!(character, '0' | '1'),
                    _ => unreachable!(),
                };
            if !digit {
                break;
            }
            end = prefix + offset + character.len_utf8();
        }
        if end == prefix {
            return Err(format!("`{literal}` is not an unsigned Rust literal"));
        }
        let suffix = &literal[end..];
        if !matches!(
            suffix,
            "" | "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize"
        ) {
            return Err(format!("unsupported Rust integer suffix `{suffix}`"));
        }
        let digits: String = literal[prefix..end]
            .chars()
            .filter(|character| *character != '_')
            .collect();
        u64::from_str_radix(&digits, radix)
            .map_err(|_| format!("Rust integer literal `{literal}` does not fit in u64"))
    }

    fn rust_delimiter_depth(source: &str, end: usize) -> [usize; 3] {
        let mut depth = [0usize; 3];
        for byte in &source.as_bytes()[..end] {
            match byte {
                b'{' => depth[0] += 1,
                b'}' => {
                    depth[0] = depth[0]
                        .checked_sub(1)
                        .expect("unmatched Rust closing brace")
                }
                b'(' => depth[1] += 1,
                b')' => {
                    depth[1] = depth[1]
                        .checked_sub(1)
                        .expect("unmatched Rust closing parenthesis")
                }
                b'[' => depth[2] += 1,
                b']' => {
                    depth[2] = depth[2]
                        .checked_sub(1)
                        .expect("unmatched Rust closing bracket")
                }
                _ => {}
            }
        }
        depth
    }

    fn immediately_preceding_outer_rust_attribute(source: &str, line_start: usize) -> bool {
        let prefix = source[..line_start].trim_end();
        let bytes = prefix.as_bytes();
        if bytes.last() != Some(&b']') {
            return false;
        }
        let mut depth = 0usize;
        for at in (0..bytes.len()).rev() {
            match bytes[at] {
                b']' => depth += 1,
                b'[' => {
                    depth = depth
                        .checked_sub(1)
                        .expect("opening Rust bracket before its close");
                    if depth == 0 {
                        return bytes.get(at.wrapping_sub(1)) == Some(&b'#');
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated Rust attribute before an ABI witness");
    }

    fn assert_unconditional_rust_line(source: &str, at: usize, depth: usize, label: &str) {
        let line_start = source[..at].rfind('\n').map_or(0, |newline| newline + 1);
        assert!(
            source[line_start..at].trim().is_empty(),
            "{label} is not a direct Rust item"
        );
        let nesting = rust_delimiter_depth(source, at);
        assert_eq!(nesting[0], depth, "{label} is at the wrong brace depth");
        assert_eq!(
            [nesting[1], nesting[2]],
            [0, 0],
            "{label} is nested in a Rust macro or expression"
        );
        assert!(
            !immediately_preceding_outer_rust_attribute(source, line_start),
            "{label} is controlled by a Rust attribute"
        );
    }

    fn rust_expression(source: &str, declaration: &str) -> String {
        let source = rust_without_comments(source);
        let prefix = format!("{declaration} = ");
        let matches: Vec<_> = source.match_indices(&prefix).collect();
        assert_eq!(matches.len(), 1, "expected one `{declaration}` declaration");
        let at = matches[0].0;
        assert_unconditional_rust_line(&source, at, 0, declaration);
        let value_start = at + prefix.len();
        let line_end = source[value_start..]
            .find('\n')
            .map_or(source.len(), |offset| value_start + offset);
        source[value_start..line_end]
            .trim()
            .strip_suffix(';')
            .unwrap_or_else(|| panic!("unterminated `{declaration}` declaration"))
            .trim()
            .to_string()
    }

    fn rust_function_body(source: &str, signature: &str) -> String {
        let source = rust_without_comments(source);
        let matches: Vec<_> = source.match_indices(signature).collect();
        assert_eq!(matches.len(), 1, "expected one `{signature}`");
        let at = matches[0].0;
        assert_unconditional_rust_line(&source, at, 0, signature);
        let open = at + signature.len() - 1;
        assert_eq!(source.as_bytes().get(open), Some(&b'{'));
        let close = matching(source.as_bytes(), open, b'{', b'}');
        source[open + 1..close].to_string()
    }

    fn rust_uint(source: &str, declaration: &str) -> u64 {
        let expression = rust_expression(source, declaration);
        parse_rust_uint(&expression)
            .unwrap_or_else(|error| panic!("unparsed `{declaration}`: {error}"))
    }

    fn rust_negated_errno(source: &str, name: &str) -> u64 {
        let expression = rust_expression(source, &format!("const {name}: u64"));
        let compact: String = expression
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let magnitude = compact
            .strip_prefix("(-")
            .and_then(|value| value.strip_suffix("i64)asu64"))
            .unwrap_or_else(|| panic!("{name} is not a negated i64 errno"));
        parse_rust_uint(magnitude).unwrap_or_else(|error| panic!("unparsed {name}: {error}"))
    }

    fn rust_status_constants(source: &str) -> std::collections::BTreeMap<String, u64> {
        let source = rust_without_comments(source);
        let marker = "STATUS_";
        let canonical = "pub(crate) const STATUS_";
        let mut statuses = std::collections::BTreeMap::new();
        let mut numbers = std::collections::BTreeSet::new();
        for (at, _) in source.match_indices(marker) {
            if rust_delimiter_depth(&source, at) != [0, 0, 0] {
                continue;
            }
            let line_start = source[..at].rfind('\n').map_or(0, |newline| newline + 1);
            assert_unconditional_rust_line(&source, line_start, 0, "kernel status constant");
            let line_end = source[at..]
                .find('\n')
                .map_or(source.len(), |offset| at + offset);
            let line = source[line_start..line_end].trim();
            let rest = line
                .strip_prefix(canonical)
                .unwrap_or_else(|| panic!("non-canonical file-scope status `{line}`"));
            let (name, expression) = rest
                .split_once(": u32 = ")
                .unwrap_or_else(|| panic!("malformed kernel status `{line}`"));
            assert!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_'),
                "malformed kernel status name `{name}`"
            );
            let expression = expression
                .strip_suffix(';')
                .unwrap_or_else(|| panic!("unterminated kernel status {name}"));
            let number = parse_rust_uint(expression)
                .unwrap_or_else(|error| panic!("unparsed kernel status {name}: {error}"));
            assert!(numbers.insert(number), "two statuses use {number}");
            assert!(
                statuses.insert(name.to_string(), number).is_none(),
                "two kernel status constants are named {name}"
            );
        }
        assert!(!statuses.is_empty(), "the kernel defines no statuses");
        statuses
    }

    fn simple_decoder_arms(
        source: &str,
        owner: &str,
        signature: &str,
        unknown: &str,
    ) -> std::collections::BTreeMap<u64, String> {
        let source = rust_without_comments(source);
        let impl_marker = format!("impl {owner} {{");
        let implementations: Vec<_> = source.match_indices(&impl_marker).collect();
        assert_eq!(
            implementations.len(),
            1,
            "expected one direct `impl {owner}`"
        );
        let impl_at = implementations[0].0;
        assert_unconditional_rust_line(&source, impl_at, 0, &format!("impl {owner}"));
        let impl_open = impl_at + impl_marker.len() - 1;
        let impl_close = matching(source.as_bytes(), impl_open, b'{', b'}');
        let signatures: Vec<_> = source.match_indices(signature).collect();
        assert_eq!(signatures.len(), 1, "expected one `{signature}`");
        let signature_at = signatures[0].0;
        assert!(
            signature_at > impl_open && signature_at < impl_close,
            "`{signature}` is outside impl {owner}"
        );
        assert_unconditional_rust_line(&source, signature_at, 1, signature);
        assert!(
            source[impl_open + 1..signature_at].trim().is_empty(),
            "impl {owner} has an item before `{signature}`"
        );
        let open = signature_at + signature.len() - 1;
        assert_eq!(source.as_bytes().get(open), Some(&b'{'));
        let close = matching(source.as_bytes(), open, b'{', b'}');
        assert!(
            source[close + 1..impl_close].trim().is_empty(),
            "impl {owner} has an item after `{signature}`"
        );
        let body: String = source[open + 1..close]
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let arms = body
            .strip_prefix("matchvalue{")
            .and_then(|body| body.strip_suffix('}'))
            .unwrap_or_else(|| panic!("`{signature}` is not only a value match"));
        let mut decoded = std::collections::BTreeMap::new();
        let mut variants = std::collections::BTreeSet::new();
        let mut saw_unknown = false;
        let members: Vec<_> = arms
            .split(',')
            .filter(|member| !member.is_empty())
            .collect();
        for (index, member) in members.iter().enumerate() {
            if *member == unknown {
                assert!(!saw_unknown, "duplicate unknown decoder arm");
                assert_eq!(index + 1, members.len(), "unknown decoder arm is not last");
                saw_unknown = true;
                continue;
            }
            assert!(!saw_unknown, "decoder arm follows the unknown arm");
            let (number, target) = member
                .split_once("=>Self::")
                .unwrap_or_else(|| panic!("non-canonical decoder arm `{member}`"));
            assert!(
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
                "decoder pattern `{number}` is not one decimal literal"
            );
            let number = number
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("decoder number `{number}` does not fit in u64"));
            assert!(
                !target.is_empty()
                    && target
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    && target
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic),
                "decoder target `{target}` is not a plain variant"
            );
            assert!(
                variants.insert(target),
                "duplicate decoder target `{target}`"
            );
            assert!(
                decoded.insert(number, target.to_string()).is_none(),
                "duplicate decoder number {number}"
            );
        }
        assert!(saw_unknown, "decoder has no canonical unknown arm");
        assert!(!decoded.is_empty(), "decoder has no known arms");
        decoded
    }

    fn c_enum_values(source: &str, name: &str) -> std::collections::BTreeMap<String, u64> {
        let source = cpp_structure(source);
        let marker = format!("enum {name}");
        let markers: Vec<_> = source.match_indices(&marker).collect();
        assert_eq!(markers.len(), 1, "expected one C enum {name}");
        let start = markers[0].0;
        assert!(
            start == 0
                || !source.as_bytes()[start - 1].is_ascii_alphanumeric()
                    && source.as_bytes()[start - 1] != b'_',
            "C enum {name} is embedded in an identifier"
        );
        let mut open = start + marker.len();
        while source
            .as_bytes()
            .get(open)
            .is_some_and(u8::is_ascii_whitespace)
        {
            open += 1;
        }
        assert_eq!(
            source.as_bytes().get(open),
            Some(&b'{'),
            "unexpected tokens before C enum {name}'s body"
        );
        let close = matching(source.as_bytes(), open, b'{', b'}');
        let mut after = close + 1;
        while source
            .as_bytes()
            .get(after)
            .is_some_and(u8::is_ascii_whitespace)
        {
            after += 1;
        }
        assert_eq!(
            source.as_bytes().get(after),
            Some(&b';'),
            "C enum {name} is not terminated"
        );
        let mut values = std::collections::BTreeMap::new();
        let mut numbers = std::collections::BTreeSet::new();
        for member in source[open + 1..close]
            .split(',')
            .map(str::trim)
            .filter(|member| !member.is_empty())
        {
            let (member_name, expression) = member
                .split_once('=')
                .unwrap_or_else(|| panic!("implicit or malformed C enum member `{member}`"));
            let member_name = member_name.trim();
            assert!(
                !member_name.is_empty()
                    && member_name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    && member_name
                        .as_bytes()
                        .first()
                        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_'),
                "malformed C enum member `{member_name}`"
            );
            let number = parse_c_uint(expression)
                .unwrap_or_else(|error| panic!("unparsed C enum member `{member}`: {error}"));
            assert!(numbers.insert(number), "aliased C enum value {number}");
            assert!(
                values.insert(member_name.to_string(), number).is_none(),
                "duplicate C enum member `{member_name}`"
            );
        }
        assert!(!values.is_empty(), "C enum {name} has no members");
        values
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ProgramOperands {
        pops: Vec<&'static str>,
        rcx_imm8_rejections: Vec<(u8, usize)>,
        ring_bases: Vec<u64>,
        eax_immediates: Vec<u32>,
        interrupts: Vec<u8>,
        writes32: Vec<(u32, u32)>,
        writes32_from_esi: Vec<u32>,
        writes64_from_imm32: Vec<(u32, u32)>,
        short_writes32: Vec<(u8, u32)>,
        short_reads32: Vec<u8>,
        reads32: Vec<u32>,
        reads64: Vec<u32>,
        eax_imm8_comparisons: Vec<u8>,
        r8d_immediates: Vec<u32>,
        writes32_from_eax: Vec<u32>,
        fenced_timestamp_reads: usize,
        r8d_zeroes: usize,
        r8d_zero_tests: usize,
        r10d_timestamp_captures: usize,
        timestamp_subtractions: usize,
        ecx_zero_tests: usize,
        rcx_zero_tests: usize,
        ecx_esi_comparisons: usize,
    }

    fn sign_extended_imm8(value: u64) -> Option<u8> {
        i8::try_from(value)
            .ok()
            .map(|immediate| immediate.to_le_bytes()[0])
    }

    fn decode_program_operands(code: &[u8]) -> ProgramOperands {
        let mut operands = ProgramOperands {
            pops: Vec::new(),
            rcx_imm8_rejections: Vec::new(),
            ring_bases: Vec::new(),
            eax_immediates: Vec::new(),
            interrupts: Vec::new(),
            writes32: Vec::new(),
            writes32_from_esi: Vec::new(),
            writes64_from_imm32: Vec::new(),
            short_writes32: Vec::new(),
            short_reads32: Vec::new(),
            reads32: Vec::new(),
            reads64: Vec::new(),
            eax_imm8_comparisons: Vec::new(),
            r8d_immediates: Vec::new(),
            writes32_from_eax: Vec::new(),
            fenced_timestamp_reads: 0,
            r8d_zeroes: 0,
            r8d_zero_tests: 0,
            r10d_timestamp_captures: 0,
            timestamp_subtractions: 0,
            ecx_zero_tests: 0,
            rcx_zero_tests: 0,
            ecx_esi_comparisons: 0,
        };
        let mut at = 0usize;
        while at < code.len() {
            let rest = &code[at..];
            let immediate32 = |offset: usize| {
                u32::from_le_bytes(
                    rest.get(offset..offset + 4)
                        .unwrap_or_else(|| panic!("truncated imm32 at byte {at}"))
                        .try_into()
                        .expect("four immediate bytes"),
                )
            };
            let bytes = if rest.starts_with(&[0x59]) {
                operands.pops.push("rcx");
                1
            } else if rest.starts_with(&[0x58]) {
                operands.pops.push("rax");
                1
            } else if rest.starts_with(&[0x5E]) {
                operands.pops.push("rsi");
                1
            } else if rest.starts_with(&[0x48, 0x83, 0xF9]) {
                let immediate = *rest
                    .get(3)
                    .unwrap_or_else(|| panic!("truncated cmp rcx, imm8 at byte {at}"));
                let rejection = rest
                    .get(4..6)
                    .unwrap_or_else(|| panic!("cmp rcx, imm8 at byte {at} has no rejection"));
                assert_eq!(
                    rejection[0], 0x75,
                    "cmp rcx, imm8 at byte {at} is not followed by jne"
                );
                let target = i64::try_from(at).expect("program offset fits in i64")
                    + 6
                    + i64::from(rejection[1] as i8);
                let target = usize::try_from(target)
                    .unwrap_or_else(|_| panic!("version rejection before byte zero at byte {at}"));
                assert!(
                    target < code.len(),
                    "version rejection leaves the program at byte {target}"
                );
                operands.rcx_imm8_rejections.push((immediate, target));
                4
            } else if rest.starts_with(&[0x48, 0xBB]) {
                let immediate = rest
                    .get(2..10)
                    .unwrap_or_else(|| panic!("truncated mov rbx, imm64 at byte {at}"));
                operands.ring_bases.push(u64::from_le_bytes(
                    immediate.try_into().expect("eight immediate bytes"),
                ));
                10
            } else if rest.starts_with(&[0xCD]) {
                operands.interrupts.push(
                    *rest
                        .get(1)
                        .unwrap_or_else(|| panic!("truncated int imm8 at byte {at}")),
                );
                2
            } else if rest.starts_with(&[0x48, 0xC7, 0x83]) {
                operands
                    .writes64_from_imm32
                    .push((immediate32(3), immediate32(7)));
                11
            } else if rest.starts_with(&[0xC7, 0x83]) {
                operands.writes32.push((immediate32(2), immediate32(6)));
                10
            } else if rest.starts_with(&[0x89, 0xB3]) {
                operands.writes32_from_esi.push(immediate32(2));
                6
            } else if rest.starts_with(&[0x89, 0x83]) {
                operands.writes32_from_eax.push(immediate32(2));
                6
            } else if rest.starts_with(&[0x48, 0x81, 0x0C, 0x24]) {
                8
            } else if rest.starts_with(&[0x48, 0x8B, 0x8B]) {
                operands.reads64.push(immediate32(3));
                7
            } else if rest.starts_with(&[0x48, 0xC7, 0xC2]) {
                7
            } else if rest.starts_with(&[0xC7, 0x43]) {
                operands.short_writes32.push((
                    *rest
                        .get(2)
                        .unwrap_or_else(|| panic!("truncated disp8 at byte {at}")),
                    immediate32(3),
                ));
                7
            } else if rest.starts_with(&[0x8B, 0x43]) {
                operands.short_reads32.push(
                    *rest
                        .get(2)
                        .unwrap_or_else(|| panic!("truncated disp8 at byte {at}")),
                );
                3
            } else if rest.starts_with(&[0x8B, 0x8B]) {
                operands.reads32.push(immediate32(2));
                6
            } else if rest.starts_with(&[0xB8]) {
                operands.eax_immediates.push(immediate32(1));
                5
            } else if rest.starts_with(&[0x41, 0xB8]) {
                operands.r8d_immediates.push(immediate32(2));
                6
            } else if rest.starts_with(&[0xBF]) {
                5
            } else if rest.starts_with(&[0x48, 0x83, 0xF8]) {
                4
            } else if rest.starts_with(&[0x83, 0xFF]) {
                3
            } else if rest.starts_with(&[0x83, 0xF8]) {
                operands.eax_imm8_comparisons.push(
                    *rest
                        .get(2)
                        .unwrap_or_else(|| panic!("truncated cmp eax, imm8 at byte {at}")),
                );
                3
            } else if rest.starts_with(&[0x48, 0x85, 0xC9]) {
                operands.rcx_zero_tests += 1;
                3
            } else if rest.starts_with(&[0x48, 0x85, 0xC0]) || rest.starts_with(&[0x48, 0x39, 0xD1])
            {
                3
            } else if rest.starts_with(&[0x45, 0x31, 0xC0]) {
                operands.r8d_zeroes += 1;
                3
            } else if rest.starts_with(&[0x45, 0x85, 0xC0]) {
                operands.r8d_zero_tests += 1;
                3
            } else if rest.starts_with(&[0x0F, 0xAE, 0xE8, 0x0F, 0x31]) {
                operands.fenced_timestamp_reads += 1;
                5
            } else if rest.starts_with(&[0x41, 0x89, 0xC2]) {
                operands.r10d_timestamp_captures += 1;
                3
            } else if rest.starts_with(&[0x44, 0x29, 0xD0]) {
                operands.timestamp_subtractions += 1;
                3
            } else if rest.starts_with(&[0x31, 0xC0])
                || rest.starts_with(&[0x31, 0xFF])
                || rest.starts_with(&[0x85, 0xFF])
                || rest.starts_with(&[0xFF, 0xE0])
                || rest.starts_with(&[0x0F, 0x0B])
                || matches!(rest.first(), Some(0x74 | 0x75 | 0x78 | 0xEB))
            {
                2
            } else if rest.starts_with(&[0x85, 0xC9]) {
                operands.ecx_zero_tests += 1;
                2
            } else if rest.starts_with(&[0x39, 0xF1]) {
                operands.ecx_esi_comparisons += 1;
                2
            } else if matches!(rest.first(), Some(0x9C | 0x9D)) {
                1
            } else {
                panic!(
                    "unrecognized emitted instruction at byte {at}: {:02x?}",
                    &rest[..rest.len().min(12)]
                );
            };
            assert!(
                rest.len() >= bytes,
                "truncated emitted instruction at byte {at}"
            );
            at += bytes;
        }
        operands
    }

    fn c_struct_fields(source: &str, name: &str) -> Vec<(String, String)> {
        let (active, _) = cpp_without_comments(source);
        let structure = cpp_structure(&active);
        let marker = format!("struct {name}");
        let markers: Vec<_> = structure.match_indices(&marker).collect();
        assert_eq!(markers.len(), 1, "expected one C struct {name}");
        let start = markers[0].0;
        assert!(
            start == 0
                || !structure.as_bytes()[start - 1].is_ascii_alphanumeric()
                    && structure.as_bytes()[start - 1] != b'_',
            "C struct {name} is embedded in an identifier"
        );
        let mut open = start + marker.len();
        while structure
            .as_bytes()
            .get(open)
            .is_some_and(u8::is_ascii_whitespace)
        {
            open += 1;
        }
        assert_eq!(
            structure.as_bytes().get(open),
            Some(&b'{'),
            "unexpected tokens before C struct {name}'s body"
        );
        let close = matching(structure.as_bytes(), open, b'{', b'}');
        let mut after = close + 1;
        while structure
            .as_bytes()
            .get(after)
            .is_some_and(u8::is_ascii_whitespace)
        {
            after += 1;
        }
        assert_eq!(
            structure.as_bytes().get(after),
            Some(&b';'),
            "C struct {name} is not terminated"
        );
        let body = &active[open + 1..close];
        assert!(
            body.trim().is_empty() || body.trim_end().ends_with(';'),
            "C struct {name}'s last field is not terminated"
        );
        body.split(';')
            .filter_map(|declaration| {
                let words: Vec<_> = declaration.split_whitespace().collect();
                if words.is_empty() {
                    return None;
                }
                assert_eq!(
                    words.len(),
                    2,
                    "C struct {name} has an unparsed field `{declaration}`"
                );
                Some((words[0].to_string(), words[1].to_string()))
            })
            .collect()
    }

    fn c_typedef_kind(source: &str, name: &str) -> String {
        let (active, _) = cpp_without_comments(source);
        let structure = cpp_structure(&active);
        let mut found = Vec::new();
        for line in structure.lines() {
            let line = line.trim();
            let Some(declaration) = line.strip_prefix("typedef ") else {
                continue;
            };
            let declaration = declaration
                .strip_suffix(';')
                .unwrap_or_else(|| panic!("unterminated C typedef `{line}`"));
            let words: Vec<_> = declaration.split_whitespace().collect();
            assert_eq!(words.len(), 2, "unparsed C typedef `{line}`");
            if words[1] == name {
                found.push(words[0]);
            }
        }
        assert_eq!(found.len(), 1, "expected one C typedef {name}");
        found[0].to_string()
    }

    fn c_layout(fields: &[(String, String)]) -> std::collections::BTreeMap<String, usize> {
        let mut layout = std::collections::BTreeMap::new();
        let mut offset = 0usize;
        for (kind, name) in fields {
            let (size, alignment) = c_type_layout(kind);
            offset = (offset + alignment - 1) & !(alignment - 1);
            assert!(
                layout.insert(name.to_ascii_uppercase(), offset).is_none(),
                "duplicate C field {name}"
            );
            offset += size;
        }
        layout
    }

    fn c_type_layout(kind: &str) -> (usize, usize) {
        match kind {
            "uint32_t" => (4, 4),
            "uint64_t" => (8, 8),
            other => panic!("no checked C layout for `{other}`"),
        }
    }

    fn c_widths(fields: &[(String, String)]) -> std::collections::BTreeMap<String, usize> {
        fields
            .iter()
            .map(|(kind, name)| (name.to_ascii_uppercase(), c_type_layout(kind).0))
            .collect()
    }

    fn c_size(fields: &[(String, String)]) -> usize {
        let mut offset = 0usize;
        let mut widest = 1usize;
        for (kind, _) in fields {
            let (size, alignment) = c_type_layout(kind);
            offset = (offset + alignment - 1) & !(alignment - 1);
            offset += size;
            widest = widest.max(alignment);
        }
        (offset + widest - 1) & !(widest - 1)
    }

    fn rust_module_offsets(source: &str, name: &str) -> std::collections::BTreeMap<String, usize> {
        let source = rust_without_comments(source);
        let marker = format!("mod {name} {{");
        let markers: Vec<_> = source.match_indices(&marker).collect();
        assert_eq!(markers.len(), 1, "expected one Rust module {name}");
        let start = markers[0].0;
        assert!(
            start == 0
                || !source.as_bytes()[start - 1].is_ascii_alphanumeric()
                    && source.as_bytes()[start - 1] != b'_',
            "Rust module {name} is embedded in an identifier"
        );
        assert_unconditional_rust_line(&source, start, 0, &format!("Rust module {name}"));
        let open = start + marker.len() - 1;
        let close = matching(source.as_bytes(), open, b'{', b'}');
        let module = &source[open + 1..close];
        let mut offsets = std::collections::BTreeMap::new();
        for line in module.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let rest = line
                .strip_prefix("pub(super) const ")
                .unwrap_or_else(|| panic!("unparsed line in Rust module {name}: `{line}`"));
            let (field, value) = rest
                .split_once(": usize = ")
                .unwrap_or_else(|| panic!("unparsed offset `{line}`"));
            let value = value
                .strip_suffix(';')
                .unwrap_or_else(|| panic!("unterminated offset `{line}`"))
                .parse()
                .unwrap_or_else(|_| panic!("non-numeric offset `{line}`"));
            assert!(
                offsets.insert(field.to_string(), value).is_none(),
                "duplicate Rust offset {field}"
            );
        }
        assert!(!offsets.is_empty(), "Rust module {name} has no offsets");
        offsets
    }

    fn rust_field_widths(source: &str, module: &str) -> std::collections::BTreeMap<String, usize> {
        let source = rust_without_comments(source);
        let mut widths = std::collections::BTreeMap::new();
        let marker = format!("{module}::");
        let identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        for (operation, width) in [
            ("read_u32_volatile", 4usize),
            ("write_u32_volatile", 4),
            ("load_index", 4),
            ("publish_index", 4),
            ("read_u32", 4usize),
            ("write_u32", 4),
            ("read_u64", 8),
            ("write_u64", 8),
            ("read_entry_u32", 4),
            ("write_entry_u32", 4),
            ("read_entry_u64", 8),
            ("write_entry_u64", 8),
        ] {
            for (at, _) in source.match_indices(operation) {
                if at != 0 && identifier(source.as_bytes()[at - 1]) {
                    continue;
                }
                let mut open = at + operation.len();
                if source
                    .as_bytes()
                    .get(open)
                    .is_some_and(|byte| identifier(*byte))
                {
                    continue;
                }
                while source
                    .as_bytes()
                    .get(open)
                    .is_some_and(u8::is_ascii_whitespace)
                {
                    open += 1;
                }
                if source.as_bytes().get(open) != Some(&b'(') {
                    continue;
                }
                let close = matching(source.as_bytes(), open, b'(', b')');
                let call = &source[open + 1..close];
                let fields: Vec<_> = call
                    .match_indices(&marker)
                    .filter(|(field_at, _)| {
                        *field_at == 0 || !identifier(call.as_bytes()[*field_at - 1])
                    })
                    .collect();
                if fields.is_empty() {
                    continue;
                }
                assert_eq!(
                    fields.len(),
                    1,
                    "{operation} call names two {module} fields"
                );
                let field_at = fields[0].0 + marker.len();
                let field: String = call[field_at..]
                    .chars()
                    .take_while(|character| character.is_ascii_uppercase() || *character == '_')
                    .collect();
                assert!(
                    !field.is_empty(),
                    "unparsed {module} field in {operation} call"
                );
                assert!(
                    call.as_bytes()
                        .get(field_at + field.len())
                        .is_none_or(|byte| !identifier(*byte)),
                    "non-canonical {module} field in {operation} call"
                );
                if let Some(previous) = widths.insert(field.clone(), width) {
                    assert_eq!(previous, width, "kernel reads {module}::{field} two ways");
                }
            }
        }
        assert!(!widths.is_empty(), "kernel never accesses {module} fields");
        widths
    }

    #[test]
    fn the_header_inventory_accepts_spacing_and_rejects_hidden_definitions() {
        let header = r#"
# ifndef WIZARD_ABI_H
# define WIZARD_ABI_H
# include <stddef.h>
# include <stdint.h>
# ifdef __cplusplus
extern "C" {
# endif
#  define WIZ_VALUE ((uint64_t)0x1234ull)
enum wiz_call { WIZ_CALL_EXIT = 0 };
# ifdef __cplusplus
}
# endif
# endif
"#;
        let (_, macros) = checked_header(header);
        assert_eq!(macros["WIZ_VALUE"], "((uint64_t)0x1234ull)");
        let raw_decoy = header.replace(
            "#  define WIZ_VALUE ((uint64_t)0x1234ull)",
            "const char *decoy = R\"wiz(\n#  define WIZ_VALUE ((uint64_t)0x1234ull)\n)wiz\";",
        );
        let (_, raw_decoy_macros) = checked_header(&raw_decoy);
        assert!(
            !raw_decoy_macros.contains_key("WIZ_VALUE"),
            "a raw-string directive entered the active macro inventory"
        );

        for corrupted in [
            header.replace(
                "#  define WIZ_VALUE ((uint64_t)0x1234ull)",
                "# if 0\n#  define WIZ_VALUE ((uint64_t)0x1234ull)\n# endif",
            ),
            header.replace("enum wiz_call", "# define WIZ_VALUE 1u\nenum wiz_call"),
            header.replace("#  define WIZ_VALUE", "%:define WIZ_VALUE"),
            header.replace("enum wiz_call", "_Pragma(\"pack(1)\")\nenum wiz_call"),
        ] {
            assert!(
                std::panic::catch_unwind(|| checked_header(&corrupted)).is_err(),
                "a non-canonical header scaffold was accepted"
            );
        }
    }

    #[test]
    fn the_c_integer_parser_admits_only_exact_unsigned_values() {
        assert_eq!(parse_c_uint("((wiz_handle_t)0)"), Ok(0));
        assert_eq!(parse_c_uint("((wiz_rights_t)(1u << 14))"), Ok(1 << 14));
        assert_eq!(
            parse_c_uint("((uintptr_t)0x0000200000000000ull)"),
            Ok(0x0000_2000_0000_0000)
        );
        for expression in [
            "08",
            "1u | 2u",
            "1u + 1u",
            "1u << 32",
            "0xffffffffu << 1",
            "WIZ_OTHER",
            "((uint32_t)1)",
        ] {
            assert!(
                parse_c_uint(expression).is_err(),
                "accepted non-restricted C integer {expression}"
            );
        }
    }

    #[test]
    fn decoder_inventory_rejects_aliases_and_noncanonical_arms() {
        let signature = "const fn decode(value: u32) -> Self {";
        let unknown = "other=>Self::Unknown(other)";
        let valid = r#"
impl Opcode {
    const fn decode(value: u32) -> Self {
        match value {
            1 => Self::Echo,
            2 => Self::Uptime,
            other => Self::Unknown(other),
        }
    }
}
"#;
        assert_eq!(
            simple_decoder_arms(valid, "Opcode", signature, unknown).len(),
            2
        );
        for corrupted in [
            valid.replace("2 => Self::Uptime", "1 => Self::Uptime"),
            valid.replace("2 => Self::Uptime", "2 => Self::Echo"),
            valid.replace("2 => Self::Uptime", "1 | 2 => Self::Uptime"),
            valid.replace("2 => Self::Uptime", "2 => { Self::Uptime }"),
            format!("#[cfg(any())]\n{valid}"),
            format!("macro_rules! decoy (\n() => (\n{valid}\n);\n);\n"),
        ] {
            assert!(
                std::panic::catch_unwind(|| {
                    simple_decoder_arms(&corrupted, "Opcode", signature, unknown)
                })
                .is_err(),
                "accepted a decoder alias or noncanonical arm"
            );
        }
    }

    #[test]
    fn c_declaration_inventory_rejects_decoys_duplicates_and_missing_terminators() {
        let source = r#"
const char *decoy = "struct wire { uint64_t wrong; };";
typedef uint32_t wire_word_t;
struct wire {
    uint32_t first;
    uint64_t second;
};
enum wiz_call {
    WIZ_CALL_EXIT = 0,
    WIZ_CALL_WAIT = 1,
};
"#;
        assert_eq!(
            c_struct_fields(source, "wire"),
            [
                ("uint32_t".to_string(), "first".to_string()),
                ("uint64_t".to_string(), "second".to_string()),
            ]
        );
        assert_eq!(c_typedef_kind(source, "wire_word_t"), "uint32_t");
        assert_eq!(c_enum_values(source, "wiz_call").len(), 2);
        for corrupted in [
            format!("{source}\nstruct wire {{ uint32_t third; }};"),
            source.replace("uint64_t second;", "uint64_t second"),
            source.replace("WIZ_CALL_WAIT = 1", "WIZ_CALL_WAIT = 0"),
            format!("{source}\ntypedef uint64_t wire_word_t;"),
        ] {
            let struct_rejected =
                std::panic::catch_unwind(|| c_struct_fields(&corrupted, "wire")).is_err();
            let enum_rejected =
                std::panic::catch_unwind(|| c_enum_values(&corrupted, "wiz_call")).is_err();
            let typedef_rejected =
                std::panic::catch_unwind(|| c_typedef_kind(&corrupted, "wire_word_t")).is_err();
            assert!(
                struct_rejected || enum_rejected || typedef_rejected,
                "a malformed C declaration was accepted"
            );
        }
    }

    #[test]
    fn rust_offset_inventory_rejects_conditional_or_duplicate_modules() {
        let module = r#"
mod submission {
    pub(super) const OPCODE: usize = 0;
    pub(super) const TOKEN: usize = 4;
}
"#;
        assert_eq!(rust_module_offsets(module, "submission").len(), 2);
        for corrupted in [
            format!("#[cfg(any())]\n{module}"),
            format!("{module}\n{module}"),
            module.replace("TOKEN: usize = 4;", "TOKEN: usize = 2 + 2;"),
            format!("macro_rules! decoy (\n() => (\n{module}\n);\n);\n"),
        ] {
            assert!(
                std::panic::catch_unwind(|| rust_module_offsets(&corrupted, "submission")).is_err(),
                "a conditional, duplicate or computed offset module was accepted"
            );
        }
    }

    #[test]
    fn rust_field_width_inventory_follows_balanced_multiline_calls() {
        let source = r#"
fn live() {
    read_u32(
        page,
        header::VERSION,
    );
    write_entry_u64(
        bytes,
        submission::ARGUMENT,
        value,
    );
}
"#;
        assert_eq!(rust_field_widths(source, "header")["VERSION"], 4);
        assert_eq!(rust_field_widths(source, "submission")["ARGUMENT"], 8);
        let conflicting = source.replace("read_u32", "read_u64") + source;
        assert!(
            std::panic::catch_unwind(|| rust_field_widths(&conflicting, "header")).is_err(),
            "line-broken accesses hid conflicting field widths"
        );
    }

    #[test]
    fn rust_integer_witnesses_must_be_unconditional_file_items() {
        let declaration = "pub const VALUE: u64";
        let valid = "pub const VALUE: u64 = 0x20;\n";
        assert_eq!(rust_uint(valid, declaration), 0x20);
        for corrupted in [
            format!("#[cfg(any())]\n{valid}"),
            format!("mod decoy {{\n    {valid}}}\n"),
            format!("{valid}{valid}"),
            valid.trim_end_matches(";\n").to_string(),
            format!("macro_rules! decoy (\n() => (\n{valid}\n);\n);\n"),
        ] {
            assert!(
                std::panic::catch_unwind(|| rust_uint(&corrupted, declaration)).is_err(),
                "accepted a conditional, nested, duplicate or unterminated Rust integer"
            );
        }
    }

    #[test]
    fn rust_status_inventory_rejects_hidden_or_noncanonical_items() {
        let valid = concat!(
            "pub(crate) const STATUS_OK: u32 = 0;\n",
            "pub(crate) const STATUS_BAD: u32 = 1;\n",
        );
        assert_eq!(rust_status_constants(valid).len(), 2);
        for corrupted in [
            valid.replace("pub(crate) const STATUS_BAD", "const STATUS_BAD"),
            valid.replace("const STATUS_BAD", "const\nSTATUS_BAD"),
            format!("#[cfg(any())]\n{valid}"),
            format!("macro_rules! decoy (\n() => (\n{valid}\n);\n);\n"),
            valid.replace("STATUS_BAD: u32 = 1", "STATUS_BAD: u32 = 0"),
        ] {
            assert!(
                std::panic::catch_unwind(|| rust_status_constants(&corrupted)).is_err(),
                "accepted a hidden, noncanonical or aliased Rust status"
            );
        }
    }

    #[test]
    fn source_witnesses_ignore_comments_and_literals() {
        let rust = r###"
const NORMAL: &str = "DECOY_NORMAL";
const RAW: &str = r#"DECOY_RAW"#;
const BYTE: u8 = b'D';
// DECOY_LINE
/* DECOY_BLOCK */
const LIVE: u32 = 7;
"###;
        let active = rust_without_comments(rust);
        for decoy in ["DECOY_NORMAL", "DECOY_RAW", "DECOY_LINE", "DECOY_BLOCK"] {
            assert!(
                !active.contains(decoy),
                "Rust literal/comment decoy survived"
            );
        }
        assert!(active.contains("const LIVE: u32 = 7;"));

        let cpp = "auto a = R\"tag(DECOY_RAW // not a comment)tag\"; // DECOY_LINE\nint live;";
        let (active, comments) = cpp_without_comments(cpp);
        assert_eq!(comments, ["DECOY_LINE"]);
        let structure = cpp_structure(&active);
        assert!(!structure.contains("DECOY_RAW"));
        assert!(structure.contains("int live;"));
    }

    #[test]
    fn emitted_operand_decoder_skips_opcode_bytes_inside_immediates() {
        let ring = 0x0000_2000_0000_0000u64;
        let mut code = vec![0xB8, 0xCD, 0x81, 0, 0, 0xCD, 0x80, 0x48, 0xBB];
        code.extend_from_slice(&ring.to_le_bytes());
        let operands = decode_program_operands(&code);
        assert_eq!(operands.eax_immediates, [0x0000_81CD]);
        assert_eq!(operands.interrupts, [0x80]);
        assert_eq!(operands.ring_bases, [ring]);
        assert!(
            std::panic::catch_unwind(|| decode_program_operands(&[0x90])).is_err(),
            "the restricted decoder accepted an unregistered instruction"
        );
    }

    #[test]
    fn emitted_operand_decoder_tracks_the_entry_handle() {
        let code = [
            0x59, 0x48, 0x83, 0xF9, 0x01, 0x75, 0x0F, 0x58, 0x5E, 0x89, 0xB3, 0x08, 0x10, 0, 0,
            0x85, 0xC9, 0x48, 0x85, 0xC9, 0x39, 0xF1, 0x0F, 0x0B,
        ];
        let operands = decode_program_operands(&code);
        assert_eq!(operands.pops, ["rcx", "rax", "rsi"]);
        assert_eq!(operands.rcx_imm8_rejections, [(1, 22)]);
        assert_eq!(&code[operands.rcx_imm8_rejections[0].1..], &[0x0F, 0x0B]);
        assert_eq!(operands.writes32_from_esi, [0x1008]);
        assert_eq!(operands.ecx_zero_tests, 1);
        assert_eq!(operands.rcx_zero_tests, 1);
        assert_eq!(operands.ecx_esi_comparisons, 1);

        let narrowed = [0x59, 0x83, 0xF9, 0x01];
        assert!(
            std::panic::catch_unwind(|| decode_program_operands(&narrowed)).is_err(),
            "a 32-bit comparison accepted a version word with unchecked high bits"
        );
    }

    #[test]
    fn entry_version_must_fit_the_sign_extended_immediate() {
        assert_eq!(sign_extended_imm8(1), Some(1));
        assert_eq!(sign_extended_imm8(127), Some(127));
        assert_eq!(sign_extended_imm8(128), None);
        assert_eq!(sign_extended_imm8(u64::MAX), None);
    }

    #[test]
    fn clock_arithmetic_widens_before_multiplication() {
        let source = r#"
            uint64_t ticks = stamp - last;
            unsigned __int128 advanced =
                static_cast<unsigned __int128>(fraction)
                + static_cast<unsigned __int128>(ticks) * multiplier;
            unsigned __int128 whole = advanced >> shift;

            uint64_t ns;
            if(whole > UINT64_MAX) {
                ns = UINT64_MAX;
            } else {
                uint64_t delta = static_cast<uint64_t>(whole);
                ns = base > UINT64_MAX - delta ? UINT64_MAX : base + delta;
            }
            *secs = 0;
        "#;
        assert_clock_arithmetic(source);

        let narrowed = source.replace(
            "static_cast<unsigned __int128>(ticks) * multiplier",
            "ticks * multiplier",
        );
        assert!(
            std::panic::catch_unwind(|| assert_clock_arithmetic(&narrowed)).is_err(),
            "a 64-bit multiply was accepted merely because its destination is 128-bit"
        );
    }

    /// Every active sysdep definition has one registered and truthful body state.
    #[test]
    fn every_sysdep_definition_agrees_with_its_register_row() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let sysdeps = std::fs::read_to_string(root.join("userland/sysdeps/wizard/generic.cpp"))
            .expect("generic.cpp");
        let readme = std::fs::read_to_string(root.join("userland/README.md")).expect("README.md");
        let (active, comments) = cpp_without_comments(&sysdeps);
        let structure = cpp_structure(&active);
        let mut includes = Vec::new();
        for (line, structural_line) in active.lines().zip(structure.lines()) {
            let structural_line = structural_line.trim_start();
            assert!(
                !structural_line.starts_with("%:"),
                "the pressure-test transcription uses a digraph directive `{line}`"
            );
            assert!(
                !structural_line
                    .split(|character: char| {
                        !character.is_ascii_alphanumeric() && character != '_'
                    })
                    .any(|word| matches!(word, "_Pragma" | "__pragma")),
                "the pressure-test transcription uses a pragma operator `{line}`"
            );
            if cpp_directive(structural_line).is_none() {
                continue;
            }
            let (kind, rest) = cpp_directive(line).expect("structural directive has source");
            assert_eq!(
                kind, "include",
                "the pressure-test transcription permits includes, not `{line}`"
            );
            includes.push(rest);
        }
        assert_eq!(
            includes,
            [
                "<mlibc/all-sysdeps.hpp>",
                "<mlibc/debug.hpp>",
                "<stdint.h>",
                "<time.h>",
                "\"wizard_abi.h\"",
            ],
            "the pressure-test transcription's include set changed"
        );
        let annotations = sysdep_annotations(&comments);
        let bodies = cpp_bodies(&active);
        let definition_names: std::collections::BTreeSet<_> = bodies
            .keys()
            .filter(|name| name.starts_with("sys_"))
            .cloned()
            .collect();
        assert_eq!(
            definition_names.len(),
            16,
            "D-114 inspected sixteen active sysdep definitions"
        );
        assert_eq!(
            definition_names,
            annotations.keys().cloned().collect(),
            "a definition is unregistered or a marker has no active definition"
        );

        let mut register = std::collections::BTreeMap::new();
        for line in readme.lines() {
            let cells: Vec<_> = line.split('|').map(str::trim).collect();
            if cells.len() < 5 {
                continue;
            }
            let name = cells[1].trim_matches('`');
            if !name.starts_with("sys_") {
                continue;
            }
            assert!(
                register
                    .insert(
                        name.to_string(),
                        (cells[2].to_string(), cells[3].to_string())
                    )
                    .is_none(),
                "{name} has two README rows"
            );
        }
        assert_eq!(
            register, annotations,
            "the README and C++ register disagree"
        );

        let vectors = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/arch/x86_64/vector.rs"))
                .expect("vector.rs"),
        );
        let syscall = u8::try_from(rust_uint(&vectors, "pub(crate) const SYSCALL: u8"))
            .expect("the syscall vector fits in an interrupt immediate");
        assert_eq!(
            compact_cpp(bodies["call1"].0),
            "inlinelongcall1(longnumber,longa)",
            "call1's declarator is not the exact native-width gate"
        );
        assert_eq!(
            compact_cpp(bodies["call1"].1),
            format!(
                "longresult;asmvolatile(\"int${syscall:#x}\":\"=a\"(result):\"a\"(number),\"D\"(a):\"memory\");returnresult;"
            ),
            "call1 is not the exact one-argument native gate"
        );
        assert_eq!(
            compact_cpp(bodies["as_errno"].0),
            "inlineintas_errno(longresult)",
            "as_errno's declarator does not preserve signed native results"
        );
        assert_eq!(
            compact_cpp(bodies["as_errno"].1),
            "returnresult<0?static_cast<int>(-result):0;",
            "as_errno does not decode exactly the native negative-result convention"
        );

        for (name, (body_state, _)) in &annotations {
            let implementation = bodies
                .get(name)
                .unwrap_or_else(|| panic!("no active definition for {name}"))
                .1;
            match body_state.as_str() {
                "WIRED" if name == "sys_exit" => assert_eq!(
                    compact_cpp(implementation),
                    "call1(WIZ_CALL_EXIT,status);__builtin_unreachable();"
                ),
                "WIRED" if name == "sys_tcb_set" => assert_eq!(
                    compact_cpp(implementation),
                    "returnas_errno(call1(WIZ_CALL_TCB_SET,reinterpret_cast<long>(pointer)));"
                ),
                "COMPOSED" => {
                    assert_eq!(name, "sys_libc_panic");
                    assert_eq!(
                        compact_cpp(implementation),
                        "sys_libc_log(\"mlibc:panic\\n\");sys_exit(127);__builtin_unreachable();"
                    );
                }
                "PARTIAL" => assert_eq!(name, "sys_clock_get"),
                "STUB" if name == "sys_libc_log" => {
                    assert_eq!(compact_cpp(implementation), "(void)message;");
                }
                "STUB" => {
                    let compact = compact_cpp(implementation);
                    assert!(
                        compact.ends_with(';'),
                        "{name}'s stub has a partial statement"
                    );
                    let statements: Vec<_> = compact
                        .split(';')
                        .filter(|statement| !statement.is_empty())
                        .collect();
                    assert_eq!(
                        statements.last(),
                        Some(&"returnWIZ_ENOSYS"),
                        "{name} is not an explicit one-path stub"
                    );
                    for statement in &statements[..statements.len() - 1] {
                        let argument = statement.strip_prefix("(void)").unwrap_or_else(|| {
                            panic!("{name}'s stub executes `{statement}` before refusing")
                        });
                        assert!(
                            !argument.is_empty()
                                && argument
                                    .bytes()
                                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
                            "{name}'s discard `{statement}` is not one parameter"
                        );
                    }
                }
                state => panic!("{name} has unproved BODY={state}"),
            }
        }

        let clock = &cpp_structure(bodies["sys_clock_get"].1);
        for required in [
            "clock != CLOCK_MONOTONIC",
            "return WIZ_EINVAL;",
            "constants->version != WIZ_TIME_VERSION",
            "WIZ_TIME_CLOCK_AT + sizeof(wiz_time_clock)",
            "!constants->counter_hz",
            "before != after || (after & 1u)",
            "!multiplier || shift > 63",
            "fraction >= (uint64_t{1} << shift)",
            "uint64_t ticks = stamp - last;",
            "unsigned __int128 advanced",
            "unsigned __int128 whole",
            "whole > UINT64_MAX",
            "base > UINT64_MAX - delta",
            "ns = UINT64_MAX;",
            "ns = base > UINT64_MAX - delta ? UINT64_MAX : base + delta;",
            "*secs = static_cast<time_t>(ns / 1000000000ull);",
            "*nanos = static_cast<long>(ns % 1000000000ull);",
            "return 0;",
            "monotonic_fallback(secs, nanos)",
        ] {
            assert!(clock.contains(required), "clock reader lost `{required}`");
        }
        assert_clock_arithmetic(clock);
        let before = clock.find("uint32_t before").expect("clock sequence start");
        let stamp = clock.find("read_counter()").expect("clock counter read");
        let after = clock.find("uint32_t after").expect("clock sequence end");
        assert!(
            before < stamp && stamp < after,
            "the counter read escaped the sequence window"
        );
        let seconds = clock.find("*secs =").expect("clock seconds store");
        let nanos = clock.find("*nanos =").expect("clock nanoseconds store");
        let success = clock.find("return 0;").expect("clock success return");
        assert!(
            after < seconds && seconds < nanos && nanos < success,
            "the clock returns success before publishing its result"
        );
        assert_eq!(
            clock
                .matches("__atomic_thread_fence(__ATOMIC_ACQUIRE);")
                .count(),
            2,
            "the sequence snapshot needs a compiler fence on both sides"
        );
        let counter = bodies["read_counter"].1;
        assert!(
            counter.contains("lfence\\n\\trdtsc\\n\\tlfence"),
            "the userspace counter read is not fenced on both sides"
        );
        let counter = compact_cpp(counter);
        assert_eq!(
            counter,
            "uint32_tlow;uint32_thigh;asmvolatile(\"lfence\\n\\trdtsc\\n\\tlfence\":\"=a\"(low),\"=d\"(high)::\"memory\");return(static_cast<uint64_t>(high)<<32)|low;",
            "the counter reader is not the exact fenced TSC transcription"
        );
        let counter_structure = compact_cpp(&cpp_structure(bodies["read_counter"].1));
        for dead in ["if(false)", "if(0)", "while(false)", "while(0)"] {
            assert!(
                !counter_structure.contains(dead),
                "the counter reader contains a literal dead wrapper `{dead}`"
            );
        }
        let counter_returns: Vec<_> = counter_structure
            .match_indices("return")
            .map(|(at, _)| {
                counter_structure[at..]
                    .split_once(';')
                    .map(|(statement, _)| format!("{statement};"))
                    .expect("terminated counter return")
            })
            .collect();
        assert_eq!(
            counter_returns,
            ["return(static_cast<uint64_t>(high)<<32)|low;"],
            "the counter reader has an early return"
        );

        let clock_structure = compact_cpp(clock);
        assert_eq!(
            clock_structure.matches("ticks=").count(),
            1,
            "the measured counter delta is overwritten"
        );
        for dead in ["if(false)", "if(0)", "while(false)", "while(0)"] {
            assert!(
                !clock_structure.contains(dead),
                "the clock reader contains a literal dead path `{dead}`"
            );
        }
        assert!(
            clock_structure
                .starts_with("if(clock!=CLOCK_MONOTONIC)returnWIZ_EINVAL;auto*constants="),
            "the clock reader does not begin with its clock-domain guard"
        );
        assert!(
            !clock_structure.contains("goto") && !clock_structure.contains("__builtin_unreachable"),
            "the clock reader bypasses its structured success path"
        );
        let clock_returns: Vec<_> = clock_structure
            .match_indices("return")
            .map(|(at, _)| {
                clock_structure[at..]
                    .split_once(';')
                    .map(|(statement, _)| format!("{statement};"))
                    .expect("terminated clock return")
            })
            .collect();
        assert_eq!(
            clock_returns,
            [
                "returnWIZ_EINVAL;",
                "returnmonotonic_fallback(secs,nanos);",
                "returnmonotonic_fallback(secs,nanos);",
                "return0;",
                "returnmonotonic_fallback(secs,nanos);",
            ],
            "the clock reader gained an unaccounted return path"
        );
        assert_eq!(
            compact_cpp(bodies["monotonic_fallback"].1),
            "(void)secs;(void)nanos;returnWIZ_ENOSYS;",
            "the unfinished Uptime route does not refuse on one path"
        );
    }

    #[test]
    fn disputed_sysdep_remainders_have_platform_witnesses() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let sysdeps = std::fs::read_to_string(root.join("userland/sysdeps/wizard/generic.cpp"))
            .expect("generic.cpp");
        let (_, comments) = cpp_without_comments(&sysdeps);
        let annotations = sysdep_annotations(&comments);
        let remainder = |name: &str| {
            annotations
                .get(name)
                .unwrap_or_else(|| panic!("no marker for {name}"))
                .1
                .as_str()
        };

        let queue = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/queue.rs")).expect("queue.rs"),
        );
        let vfs = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/vfs.rs")).expect("vfs.rs"),
        );
        assert_eq!(remainder("sys_read"), "COMPAT");
        assert_eq!(remainder("sys_seek"), "COMPAT");
        assert!(queue.contains("let offset = u32::try_from(submission.offset)"));
        assert!(vfs.contains("pub(crate) fn read(file: File, offset: u32,"));

        assert_eq!(remainder("sys_open"), "PARTIAL");
        assert!(queue.contains("6 => Self::OpenAt"));
        assert!(queue.contains("table.resolve(submission.handle)"));
        assert!(!queue.contains("Self::ChannelReceive"));

        let handles = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/handle.rs")).expect("handle.rs"),
        );
        let objects = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/object.rs")).expect("object.rs"),
        );
        assert_eq!(remainder("sys_close"), "PARTIAL");
        assert!(handles.contains("pub(crate) fn close(&self, handle: u32)"));
        assert!(handles.contains("drop(released);"));
        assert!(objects.contains("Shared(Arc<Object>)"));
        assert!(!queue.contains("=> Self::Close"));

        assert_eq!(remainder("sys_write"), "MISSING");
        assert!(queue.contains("7 => Self::Read"));
        assert!(!queue.contains("=> Self::Write"));

        let wait = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/wait.rs")).expect("wait.rs"),
        );
        assert_eq!(remainder("sys_futex_wait"), "MISSING");
        assert_eq!(remainder("sys_futex_wake"), "MISSING");
        assert!(wait.contains("wait_unless(&self, context: &mut ThreadContext, ready:"));
        assert!(!wait.contains("user_address"));
        assert!(!queue.contains("Self::Futex"));

        let paging = rust_without_comments(
            &std::fs::read_to_string(root.join("wost/src/arch/x86_64/paging.rs"))
                .expect("paging.rs"),
        );
        let memory = rust_without_comments(
            &std::fs::read_to_string(root.join("wost/src/mm/mod.rs")).expect("mm/mod.rs"),
        );
        assert_eq!(remainder("sys_vm_unmap"), "PARTIAL");
        assert_eq!(remainder("sys_anon_free"), "PARTIAL");
        assert!(paging.contains("pub fn unmap(&mut self, virtual_base: u64, len: u64)"));
        assert!(paging.contains("impl Drop for Unmapped"));
        assert!(paging.contains("if frames.zero()"));
        assert!(memory.contains("impl Drop for Segment"));
        assert!(memory.contains("FRAMES.lock_leaf().free_contiguous(self.first, self.count)"));
        assert!(!queue.contains("=> Self::Unmap"));
        assert!(!queue.contains("=> Self::AnonFree"));

        let user = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/user.rs")).expect("user.rs"),
        );
        assert_eq!(remainder("sys_libc_panic"), "PARTIAL");
        assert_eq!(remainder("sys_exit"), "PARTIAL");
        let exit = user
            .split_once("Call::Exit => {")
            .and_then(|(_, source)| source.split_once("Call::RingEnter =>"))
            .map(|(body, _)| body)
            .expect("Call::Exit arm");
        assert!(exit.contains("thread::retire_from_trap(frame)"));
        assert!(!exit.contains("frame.rdi"));
    }

    /// The C pressure-test transcription agrees with the active Rust ABI.
    ///
    /// This discovers the complete call and opcode domains, inventories every
    /// `WIZ_*` macro, checks the named wire declarations, and derives their
    /// offsets, widths and sizes. The emitted ring program is compared through
    /// its actual instruction operands rather than source constants (D-181).
    #[test]
    fn the_c_header_and_the_kernel_agree_about_the_abi() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let header = std::fs::read_to_string(root.join("userland/abi/wizard_abi.h"))
            .expect("userland/abi/wizard_abi.h");
        let queue = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/queue.rs")).expect("queue.rs"),
        );
        let user = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/user.rs")).expect("user.rs"),
        );
        let proto = rust_without_comments(
            &std::fs::read_to_string(root.join("libs/ring-proto/src/lib.rs")).expect("ring-proto"),
        );
        let handle_space = rust_without_comments(
            &std::fs::read_to_string(root.join("libs/handle-space/src/lib.rs"))
                .expect("handle-space"),
        );
        let x86_paging = rust_without_comments(
            &std::fs::read_to_string(root.join("libs/x86-paging/src/lib.rs")).expect("x86-paging"),
        );
        let timepage_source = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/timepage.rs")).expect("timepage.rs"),
        );
        let vectors_source = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/arch/x86_64/vector.rs"))
                .expect("vector.rs"),
        );
        let paging_source = rust_without_comments(
            &std::fs::read_to_string(root.join("wost/src/arch/x86_64/paging.rs"))
                .expect("paging.rs"),
        );
        let (active_header, macro_expressions) = checked_header(&header);
        assert_eq!(c_typedef_kind(&active_header, "wiz_handle_t"), "uint32_t");
        assert_eq!(c_typedef_kind(&active_header, "wiz_rights_t"), "uint32_t");
        assert_eq!(c_typedef_kind(&active_header, "wiz_errno_t"), "int");

        let declared_macros: std::collections::BTreeMap<String, u64> = macro_expressions
            .iter()
            .map(|(name, expression)| {
                let value = parse_c_uint(expression)
                    .unwrap_or_else(|error| panic!("unparsed {name}: {error}"));
                (name.clone(), value)
            })
            .collect();
        let mut expected_macros = std::collections::BTreeMap::new();
        macro_rules! expect_macro {
            ($name:expr, $value:expr) => {
                assert!(
                    expected_macros.insert($name.to_string(), $value).is_none(),
                    "two kernel witnesses register {}",
                    $name
                )
            };
        }

        let reserved = rust_uint(&handle_space, "pub const RESERVED: u32");
        let reserved_bits = rust_uint(&handle_space, "pub const RESERVED_BITS: u32");
        assert!(reserved_bits < u64::from(u64::BITS));
        assert_eq!(reserved, (1u64 << reserved_bits) - 1);
        let invalid = reserved & !reserved;
        assert_ne!(
            invalid & reserved,
            reserved,
            "invalid handle passes decode's low-bit gate"
        );
        expect_macro!("WIZ_HANDLE_INVALID", invalid);

        let rights_marker = "impl Rights {";
        let rights_markers: Vec<_> = handle_space.match_indices(rights_marker).collect();
        assert_eq!(rights_markers.len(), 1, "expected one impl Rights");
        let rights_at = rights_markers[0].0;
        assert_unconditional_rust_line(&handle_space, rights_at, 0, "impl Rights");
        let rights_open = rights_at + rights_marker.len() - 1;
        let rights_close = matching(handle_space.as_bytes(), rights_open, b'{', b'}');
        let rights_impl = &handle_space[rights_open + 1..rights_close];
        assert!(
            !rights_impl.contains("#["),
            "impl Rights contains a conditionally visible item"
        );
        let compact_rights: String = rights_impl
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let mut right_bits = std::collections::BTreeSet::new();
        let mut right_names = std::collections::BTreeSet::new();
        let mut at = 0usize;
        while let Some(found) = compact_rights[at..].find("pubconst") {
            let declaration_at = at + found;
            let rest = &compact_rights[declaration_at + "pubconst".len()..];
            let semicolon = rest
                .find(';')
                .unwrap_or_else(|| panic!("unterminated public item in impl Rights"));
            let declaration = &rest[..semicolon];
            at = declaration_at + "pubconst".len() + semicolon + 1;
            if declaration.starts_with("fn") {
                continue;
            }
            let (name, expression) = declaration
                .split_once(":Self=")
                .unwrap_or_else(|| panic!("unparsed public Rights item `{declaration}`"));
            if matches!(name, "NONE" | "UNIVERSAL" | "ALL") {
                continue;
            }
            assert!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_'),
                "malformed right name {name}"
            );
            assert!(right_names.insert(name), "duplicate right {name}");
            let bit = expression
                .strip_prefix("Self(1<<")
                .and_then(|value| value.strip_suffix(')'))
                .unwrap_or_else(|| panic!("right {name} is not one literal bit"));
            let bit = parse_rust_uint(bit)
                .unwrap_or_else(|error| panic!("unparsed right {name}: {error}"));
            assert!(bit < 32, "right {name} is outside wiz_rights_t");
            assert!(right_bits.insert(bit), "two rights use bit {bit}");
            expect_macro!(&format!("WIZ_RIGHT_{name}"), 1u64 << bit);
        }
        assert!(
            !right_bits.is_empty(),
            "the kernel defines no individual rights"
        );

        let ring = rust_uint(&user, "pub(crate) const RING: u64");
        let entry_version = rust_uint(&user, "pub(crate) const ENTRY_VERSION: u64");
        let entry_bytes = rust_uint(&user, "pub(crate) const ENTRY_BYTES: usize");
        let entry_layout = rust_module_offsets(&user, "entry");
        expect_macro!("WIZ_ENTRY_VERSION", entry_version);
        expect_macro!("WIZ_ENTRY_VERSION_AT", entry_layout["VERSION"] as u64);
        expect_macro!("WIZ_ENTRY_ARGUMENT_AT", entry_layout["ARGUMENT"] as u64);
        expect_macro!(
            "WIZ_ENTRY_BOOTSTRAP_HANDLE_AT",
            entry_layout["BOOTSTRAP_HANDLE"] as u64
        );
        expect_macro!("WIZ_ENTRY_BYTES", entry_bytes);
        expect_macro!("WIZ_RING_ADDRESS", ring);
        expect_macro!(
            "WIZ_RING_VERSION",
            rust_uint(&proto, "pub const VERSION: u32")
        );
        let page_size = rust_uint(&x86_paging, "pub const PAGE_SIZE: u64");
        assert_eq!(
            rust_expression(&queue, "const ENTRIES_AT: usize"),
            "PAGE_SIZE as usize"
        );
        assert_eq!(
            rust_expression(&queue, "const DATA_AT: usize"),
            "2 * PAGE_SIZE as usize"
        );
        assert_eq!(
            rust_expression(&queue, "const DATA_BYTES: usize"),
            "PAGE_SIZE as usize"
        );
        expect_macro!("WIZ_RING_ENTRIES_AT", page_size);
        expect_macro!(
            "WIZ_RING_DATA_AT",
            page_size.checked_mul(2).expect("two pages fit in u64")
        );
        expect_macro!("WIZ_RING_DATA_BYTES", page_size);

        let decoded_opcode_arms = simple_decoder_arms(
            &queue,
            "Opcode",
            "const fn decode(value: u32) -> Self {",
            "other=>Self::Unknown(other)",
        );
        for (number, variant) in &decoded_opcode_arms {
            expect_macro!(&format!("WIZ_OP_{}", variant.to_ascii_uppercase()), *number);
        }
        let decoded_call_arms = simple_decoder_arms(
            &user,
            "Call",
            "const fn decode(value: u64) -> Self {",
            "other=>Self::Unknown(other)",
        );

        for (name, number) in rust_status_constants(&queue) {
            expect_macro!(&format!("WIZ_STATUS_{name}"), number);
        }

        expect_macro!(
            "WIZ_SUBMISSION_BYTES",
            rust_uint(&queue, "const SUBMISSION_BYTES: usize")
        );
        expect_macro!(
            "WIZ_COMPLETION_BYTES",
            rust_uint(&queue, "const COMPLETION_BYTES: usize")
        );
        expect_macro!(
            "WIZ_TIME_ADDRESS",
            rust_uint(&timepage_source, "pub(crate) const TIME: u64")
        );
        expect_macro!(
            "WIZ_TIME_VERSION",
            rust_uint(&timepage_source, "pub(crate) const VERSION: u32")
        );
        expect_macro!(
            "WIZ_TIME_CLOCK_AT",
            rust_uint(&timepage_source, "const CLOCK_AT: usize")
        );
        expect_macro!(
            "WIZ_TIME_MODE_NONE",
            rust_uint(&timepage_source, "pub(crate) const MODE_NONE: u32")
        );
        expect_macro!(
            "WIZ_TIME_MODE_COUNTER",
            rust_uint(&timepage_source, "pub(crate) const MODE_COUNTER: u32")
        );
        let syscall_number = rust_uint(&vectors_source, "pub(crate) const SYSCALL: u8");
        expect_macro!("WIZ_SYSCALL_VECTOR", syscall_number);
        expect_macro!(
            "WIZ_USER_END",
            rust_uint(&paging_source, "pub const USER_END: u64")
        );
        expect_macro!("WIZ_ENOSYS", rust_negated_errno(&user, "ENOSYS"));
        expect_macro!("WIZ_EINVAL", rust_negated_errno(&user, "EINVAL"));
        assert_eq!(
            declared_macros, expected_macros,
            "the complete C macro inventory differs from the kernel ABI"
        );

        // The complete opcode domain, discovered on both sides. Checking seven
        // named members would accept an eighth kernel operation missing from
        // the C consumer just as readily as it once accepted speculative C
        // operations missing from the kernel.
        let declared_opcodes: std::collections::BTreeMap<_, _> = declared_macros
            .iter()
            .filter(|(name, _)| name.starts_with("WIZ_OP_"))
            .map(|(name, number)| (name.clone(), *number))
            .collect();
        let decoded_opcodes: std::collections::BTreeMap<_, _> = decoded_opcode_arms
            .iter()
            .map(|(number, variant)| (format!("WIZ_OP_{}", variant.to_ascii_uppercase()), *number))
            .collect();
        assert_eq!(
            declared_opcodes, decoded_opcodes,
            "the C opcode domain differs from Opcode::decode"
        );

        // The complete entry layouts. Kernel parsing, serialization and test
        // support all consume these offset modules; the C declarations and
        // emitted instructions are independent transcriptions.
        let process_entry_fields = c_struct_fields(&header, "wiz_entry_block");
        assert_eq!(
            c_layout(&process_entry_fields),
            entry_layout,
            "the C process-entry block differs from the kernel's field offsets"
        );
        assert!(
            c_widths(&process_entry_fields)
                .values()
                .all(|width| *width == 8),
            "every process-entry field is one u64 word"
        );
        assert_eq!(
            c_size(&process_entry_fields) as u64,
            entry_bytes,
            "the C process-entry block differs from the kernel's byte extent"
        );
        assert_eq!(
            rust_expression(&user, "const ENTRY_STACK_BYTES: u64"),
            "(ENTRY_BYTES as u64 + 15) & !15",
            "the initial stack pointer is not aligned beyond the entry block"
        );
        let compact_cross = compact_cpp(&rust_function_body(&user, "fn cross() {"));
        assert!(
            compact_cross.contains("lettop=launch.stack+USER_STACK_BYTES-ENTRY_STACK_BYTES;"),
            "the kernel does not place the entry block at the initial stack pointer"
        );
        assert_eq!(
            compact_cross
                .matches("usercopy::copy_to_user(top+entry::")
                .count(),
            3,
            "the kernel does not write exactly three process-entry words"
        );
        for write in [
            "usercopy::copy_to_user(top+entry::VERSIONasu64,&ENTRY_VERSION.to_le_bytes()",
            "usercopy::copy_to_user(top+entry::ARGUMENTasu64,&launch.argument.to_le_bytes()",
            concat!(
                "usercopy::copy_to_user(top+entry::BOOTSTRAP_HANDLEasu64,",
                "&u64::from(launch.bootstrap_handle).to_le_bytes()"
            ),
        ] {
            assert!(
                compact_cross.contains(write),
                "the kernel does not write one u64 entry word as `{write}`"
            );
        }
        assert!(
            compact_cross.contains("task::enter_user(launch.entry,top)"),
            "the kernel enters ring 3 at a pointer other than the checked entry block"
        );

        let ring_header_fields = c_struct_fields(&header, "wiz_ring_header");
        let submission_fields = c_struct_fields(&header, "wiz_submission");
        let completion_fields = c_struct_fields(&header, "wiz_completion");
        let ring_header_layout = rust_module_offsets(&queue, "header");
        let mut c_ring_header_layout = c_layout(&ring_header_fields);
        assert_eq!(
            c_ring_header_layout.remove("RESERVED0"),
            Some(12),
            "the C ring header's reserved word moved"
        );
        assert_eq!(
            c_ring_header_layout, ring_header_layout,
            "the C ring header differs from the kernel's field offsets"
        );
        let mut c_ring_header_widths = c_widths(&ring_header_fields);
        assert_eq!(
            c_ring_header_widths.remove("RESERVED0"),
            Some(4),
            "the C ring header's reserved word changed width"
        );
        assert_eq!(
            c_ring_header_widths,
            rust_field_widths(&queue, "header"),
            "the C ring header differs from the kernel's field widths"
        );
        assert_eq!(c_size(&ring_header_fields), 32, "C ring header size");
        let submission_layout = rust_module_offsets(&queue, "submission");
        let completion_layout = rust_module_offsets(&queue, "completion");
        assert_eq!(
            c_layout(&submission_fields),
            submission_layout,
            "the C submission differs from the kernel's field offsets"
        );
        assert_eq!(
            c_widths(&submission_fields),
            rust_field_widths(&queue, "submission"),
            "the C submission differs from the kernel's field widths"
        );
        assert_eq!(
            c_layout(&completion_fields),
            completion_layout,
            "the C completion differs from the kernel's field offsets"
        );
        assert_eq!(
            c_widths(&completion_fields),
            rust_field_widths(&queue, "completion"),
            "the C completion differs from the kernel's field widths"
        );
        let submission_bytes = u32::try_from(rust_uint(&queue, "const SUBMISSION_BYTES: usize"))
            .expect("SUBMISSION_BYTES fits in u32");
        let completion_bytes = u32::try_from(rust_uint(&queue, "const COMPLETION_BYTES: usize"))
            .expect("COMPLETION_BYTES fits in u32");
        assert_eq!(c_size(&submission_fields) as u32, submission_bytes);
        assert_eq!(c_size(&completion_fields) as u32, completion_bytes);
        assert_eq!(submission_bytes, 32, "submission wire size");
        assert_eq!(completion_bytes, 16, "completion wire size");
        assert!(
            queue.contains(
                "const COMPLETIONS_AT: usize = ENTRIES_AT + SUBMISSIONS as usize * SUBMISSION_BYTES;"
            ),
            "the kernel's completion base is not derived from its submission layout"
        );

        let entries_at = u32::try_from(page_size).expect("the entry base fits in disp32");
        assert!(
            queue.contains("const ENTRIES_AT: usize = PAGE_SIZE as usize;"),
            "the kernel's entry base is not one page"
        );
        let submissions = u32::try_from(rust_uint(&queue, "const SUBMISSIONS: u32"))
            .expect("SUBMISSIONS fits in u32");
        assert_eq!(
            submissions, 8,
            "the emitted program assumes eight submissions"
        );

        let image = crate::image::program::build();
        let elf = elf64::Elf::parse(&image).expect("the emitted program is ELF64");
        let segment = elf
            .segments()
            .next()
            .expect("the emitted program has a segment");
        let start = segment.offset() as usize;
        let end = start + segment.file_size() as usize;
        let code = &image[start..end];
        let operands = decode_program_operands(code);
        assert_eq!(
            operands.pops,
            ["rcx", "rax", "rsi"],
            "the emitted program does not consume version, argument and bootstrap handle in order"
        );
        let entry_version_immediate = sign_extended_imm8(entry_version)
            .expect("the entry version fits the sign-extended cmp rcx, imm8");
        let [(compared_version, rejected_at)] = operands.rcx_imm8_rejections.as_slice() else {
            panic!("the emitted program does not compare and reject one entry-block version");
        };
        assert_eq!(
            *compared_version, entry_version_immediate,
            "the emitted program accepts a different entry-block version"
        );
        assert_eq!(
            code.get(*rejected_at..*rejected_at + 2),
            Some(&[0x0F, 0x0B][..]),
            "the emitted program does not reject an unknown entry-block version with ud2"
        );
        assert_eq!(
            operands.ring_bases,
            [ring],
            "the emitted mov rbx, imm64 does not load the kernel ring base"
        );
        let ring_enter = decoded_call_arms
            .iter()
            .find_map(|(number, variant)| (variant == "RingEnter").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Call::RingEnter fits in an emitted imm32");
        let ring_wait = decoded_call_arms
            .iter()
            .find_map(|(number, variant)| (variant == "RingWait").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Call::RingWait fits in an emitted imm32");
        assert_eq!(
            operands.eax_immediates,
            [0x0BAD_0BAD, ring_enter, ring_wait],
            "the emitted mov eax immediates are not refusal, RingEnter and RingWait"
        );
        let syscall_immediate =
            u8::try_from(syscall_number).expect("the syscall vector fits in imm8");
        assert_eq!(
            operands.interrupts,
            vec![syscall_immediate; 5],
            "an emitted int imm8 uses a vector other than the kernel syscall gate"
        );
        let completion_at = entries_at + submissions * submission_bytes;
        let echo = decoded_opcode_arms
            .iter()
            .find_map(|(number, variant)| (variant == "Echo").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Opcode::Echo fits in an emitted imm32");
        let duplicate = decoded_opcode_arms
            .iter()
            .find_map(|(number, variant)| (variant == "Duplicate").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Opcode::Duplicate fits in an emitted imm32");
        let timer = decoded_opcode_arms
            .iter()
            .find_map(|(number, variant)| (variant == "Timer").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Opcode::Timer fits in an emitted imm32");
        assert_eq!(
            operands.writes32,
            [
                (entries_at + submission_layout["TOKEN"] as u32, 0x1234),
                (entries_at + submission_layout["OPCODE"] as u32, echo),
                (entries_at + submission_layout["OPCODE"] as u32, duplicate,),
                (entries_at + submission_layout["OPCODE"] as u32, timer),
            ],
            "the emitted program's submission imm32 writes differ from the kernel layout"
        );
        assert_eq!(
            operands.writes32_from_esi,
            [entries_at + submission_layout["HANDLE"] as u32],
            "the emitted program does not submit the bootstrap handle from its entry block"
        );
        assert_eq!(
            operands.r8d_zeroes, 1,
            "the emitted program does not default to ordinary ring service"
        );
        assert_eq!(
            operands.r8d_immediates,
            [1],
            "the measured behaviour does not arm its timestamp path"
        );
        assert_eq!(
            operands.r8d_zero_tests, 2,
            "the measurement selector does not guard both timestamp reads"
        );
        assert_eq!(
            operands.fenced_timestamp_reads, 2,
            "the real RingEnter is not bracketed by two fenced timestamp reads"
        );
        assert_eq!(
            operands.r10d_timestamp_captures, 1,
            "the first timestamp is not retained across RingEnter"
        );
        assert_eq!(
            operands.timestamp_subtractions, 1,
            "the second timestamp is not reduced to the round-trip delta"
        );
        assert_eq!(
            operands.writes32_from_eax,
            [entries_at + submission_layout["OFFSET"] as u32],
            "the measured delta does not use the consumed submission's offset field"
        );
        let duplicate_rights = u32::try_from(declared_macros["WIZ_RIGHT_READ"])
            .expect("Channel READ fits in the emitted imm32");
        assert_eq!(
            operands.writes64_from_imm32,
            [
                (
                    entries_at + submission_layout["ARGUMENT"] as u32,
                    0x00C0_FFEE,
                ),
                (
                    entries_at + submission_layout["ARGUMENT"] as u32,
                    duplicate_rights,
                ),
                (entries_at + submission_layout["ARGUMENT"] as u32, 2_000_000,),
            ],
            "the emitted program's submission imm64 write differs from the kernel layout"
        );
        assert_eq!(
            operands.short_writes32,
            [
                (ring_header_layout["SUBMISSION_TAIL"] as u8, 1),
                (ring_header_layout["COMPLETION_HEAD"] as u8, 1),
            ],
            "the emitted program publishes through different header fields"
        );
        assert_eq!(
            operands.short_reads32,
            [ring_header_layout["COMPLETION_TAIL"] as u8],
            "the emitted program acquires a different header field"
        );
        assert_eq!(
            operands.eax_imm8_comparisons,
            [1],
            "the emitted program does not require one published completion"
        );
        assert_eq!(
            operands.reads32,
            [completion_at + completion_layout["STATUS"] as u32],
            "the emitted program reads completion.status at a different offset"
        );
        assert_eq!(
            operands.reads64,
            [completion_at + completion_layout["VALUE"] as u32],
            "the emitted program reads completion.value at a different offset"
        );
        assert_eq!(
            operands.ecx_zero_tests, 1,
            "the emitted program does not require a successful completion status"
        );
        assert_eq!(
            operands.rcx_zero_tests, 2,
            "the emitted program does not reject zero duplicate and Timer answers"
        );
        assert_eq!(
            operands.ecx_esi_comparisons, 1,
            "the emitted program does not distinguish the duplicate from its source handle"
        );

        // The time page. Its address is the one number that, if it drifted,
        // would have a program reading the clock out of an unmapped page — and
        // its layout is a second transcription of `timepage.rs`'s offsets,
        // which is exactly the arrangement this test exists for (D-166).
        let timepage = &timepage_source;

        // A field moved in either language is a different wire layout. Derive
        // the C offsets from its declared types and compare both complete maps.
        let constants_fields = c_struct_fields(&header, "wiz_time_constants");
        let clock_fields = c_struct_fields(&header, "wiz_time_clock");
        assert_eq!(
            c_layout(&constants_fields),
            rust_module_offsets(timepage, "constants"),
            "the C constants struct differs from the kernel's offsets"
        );
        assert_eq!(
            c_widths(&constants_fields),
            rust_field_widths(timepage, "constants"),
            "the C constants struct differs from the kernel's field widths"
        );
        assert_eq!(c_size(&constants_fields), 16, "C constants struct size");
        assert_eq!(
            c_layout(&clock_fields),
            rust_module_offsets(timepage, "clock"),
            "the C clock struct differs from the kernel's offsets"
        );
        assert_eq!(
            c_widths(&clock_fields),
            rust_field_widths(timepage, "clock"),
            "the C clock struct differs from the kernel's field widths"
        );
        assert_eq!(c_size(&clock_fields), 48, "C clock struct size");

        // Every call number, against the kernel's own decoder. Equality of
        // the whole header set matters: checking four known members did not
        // reject the eight speculative members D-181 removed.
        let checked_declared_calls = c_enum_values(&active_header, "wiz_call");
        let checked_expected_calls: std::collections::BTreeMap<_, _> = decoded_call_arms
            .iter()
            .map(|(number, variant)| {
                let name = match variant.as_str() {
                    "Exit" => "WIZ_CALL_EXIT",
                    "RingEnter" => "WIZ_CALL_RING_ENTER",
                    "ThreadBase" => "WIZ_CALL_TCB_SET",
                    "RingWait" => "WIZ_CALL_RING_WAIT",
                    other => panic!("no C spelling registered for Call::{other}"),
                };
                (name.to_string(), *number)
            })
            .collect();
        assert_eq!(
            checked_declared_calls, checked_expected_calls,
            "the exact C call-number domain differs from Call::decode"
        );

        // The boundary has one definition; usercopy only re-exports it.
        let usercopy = rust_without_comments(
            &std::fs::read_to_string(root.join("wost/src/arch/x86_64/usercopy.rs"))
                .expect("usercopy.rs"),
        );
        assert!(
            usercopy.contains("pub use super::paging::USER_END;"),
            "usercopy states the boundary again instead of re-exporting it"
        );
    }

    /// And the count the kernel declares is the number of checks it runs.
    ///
    /// `EXPECTED_CHECKS` is deliberately independent of the running tally, so
    /// the two can disagree — and the boot that finds out costs a minute
    /// (D-105).
    #[test]
    fn the_expected_count_is_the_number_of_checks_registered() {
        let selftest = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("xtask sits below the workspace root")
                .join("kernel")
                .join("src")
                .join("selftest.rs"),
        )
        .expect("selftest.rs");

        let registered = selftest.matches("report.check(").count();
        assert_eq!(
            registered, EXPECTED_SELFTEST_CHECKS,
            "the kernel registers {registered} checks and declares {EXPECTED_SELFTEST_CHECKS}"
        );
    }
}
