//! Locating and launching QEMU.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::paths::Paths;
use crate::qmp::Qmp;

/// Default install location on Windows, checked before `PATH`.
const WINDOWS_DEFAULT: &str = r"C:\Program Files\qemu";

/// The machine both runners boot.
///
/// `-cpu max` rather than QEMU's default `qemu64`, which exposes neither SMEP,
/// SMAP nor UMIP: the kernel would switch its security baseline on against a
/// CPU that cannot enforce it, and the check that it did would pass vacuously.
/// The VMware target of D-18 and the real machine both have all three.
const MACHINE: &[&str] = &[
    "-machine",
    "q35",
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

/// Boot `image` in QEMU, forwarding `extra` arguments verbatim.
pub fn run(paths: &Paths, image: &Path, extra: &[OsString]) -> Result<()> {
    let qemu_dir = locate()?;
    let code = firmware(&qemu_dir, CODE_FIRMWARE)?;

    // The variable store must be writable, and starting from the pristine
    // template every run keeps boots reproducible.
    let vars_template = firmware(&qemu_dir, VARS_FIRMWARE)?;
    let vars = paths.efi_vars();
    std::fs::copy(&vars_template, &vars)
        .with_context(|| format!("copying {} to {}", vars_template.display(), vars.display()))?;

    let emulator = qemu_dir.join(exe_name("qemu-system-x86_64"));
    println!("[xtask] launching QEMU (serial on stdio, close the window to exit)");

    let status = Command::new(&emulator)
        .args(MACHINE)
        .args(["-serial", "stdio"])
        .args(drive(&format!(
            "if=pflash,format=raw,readonly=on,file={}",
            code.display()
        )))
        .args(drive(&format!(
            "if=pflash,format=raw,file={}",
            vars.display()
        )))
        .args(drive(&format!("format=raw,file={}", image.display())))
        .args(storage(paths.root())?)
        .args(extra)
        .status()
        .with_context(|| format!("could not launch {}", emulator.display()))?;

    // A non-zero status here usually means the user closed the window, which is
    // how you quit QEMU — reporting it as a build failure would be a lie.
    if !status.success() {
        println!("[xtask] QEMU exited with {status}");
    }
    Ok(())
}

fn drive(spec: &str) -> [String; 2] {
    ["-drive".to_string(), spec.to_string()]
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
fn storage(root: &Path) -> Result<Vec<String>> {
    let backing = root.join("target").join("nvme.img");
    let sized = std::fs::metadata(&backing).is_ok_and(|found| found.len() == NVME_BYTES);
    if !sized {
        // Written whole rather than sized and left sparse: the first blocks
        // carry a signature the block-layer checks read, and a FAT32 volume
        // follows at `NVME_VOLUME_SECTOR`.
        let mut content = vec![0u8; NVME_BYTES as usize];
        for lba in 0..NVME_SIGNED_BLOCKS {
            let at = lba as usize * NVME_BLOCK_BYTES;
            content[at..at + 8].copy_from_slice(&nvme_signature(lba).to_le_bytes());
        }
        let volume = crate::image::corpus::build().context("building the corpus volume")?;
        let at = NVME_VOLUME_SECTOR as usize * NVME_BLOCK_BYTES;
        content[at..at + volume.len()].copy_from_slice(&volume);
        std::fs::write(&backing, &content)
            .with_context(|| format!("writing {}", backing.display()))?;
    }
    Ok(vec![
        "-drive".to_string(),
        format!("if=none,id=wizardnvm,format=raw,file={}", backing.display()),
        "-device".to_string(),
        "nvme,serial=wizard0,drive=wizardnvm".to_string(),
    ])
}

/// QEMU exits with `(value << 1) | 1`, so the kernel's 0x10 arrives as 33 and
/// its 0x11 as 35. Kept in step with `kernel/src/arch/x86_64/debug_exit.rs`.
const SELFTEST_PASSED: i32 = 33;
const SELFTEST_FAILED: i32 = 35;

/// The serial verdict contract shared with `kernel/src/selftest.rs`.
const SELFTEST_VERDICT_PREFIX: &str = "selftest: verdict ";
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
const EXPECTED_SELFTEST_CHECKS: usize = 173;

/// Line the self-test kernel prints immediately before its bounded wait for
/// an injected keystroke. Kept in step with `kernel/src/selftest.rs`.
const READY_MARKER: &str = "selftest: awaiting injected keystroke";

/// Line the shipped kernel prints once it has finished booting and is doing
/// the one thing it does. Kept in step with `kernel/src/main.rs`.
const SETTLED_MARKER: &str = "kernel: reading the keyboard";

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

#[derive(Debug, Default, PartialEq, Eq)]
struct SerialObservation {
    verdict: Option<SelftestVerdict>,
    /// Names of the checks that reported `FAILED`, in the order printed.
    ///
    /// Recorded so a failure can say *which* check went red rather than only
    /// that one did. `cargo xtask mutate` is the caller that needs it: a
    /// mutation reported "caught" because some check failed is a weaker claim
    /// than one caught because *its* check failed, and the two came apart in
    /// the table — mutation 4 reddens mutation 3's check, so deleting 4's own
    /// target would leave it printing "caught" for ever (D-134).
    failed: Vec<String>,
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

fn observe_selftest_line(observation: &mut SerialObservation, line: &str) -> Result<()> {
    if let Some(name) = parse_failed_check(line) {
        observation.failed.push(name.to_string());
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

fn correlate_selftest(exit_code: Option<i32>, observation: SerialObservation) -> Result<()> {
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

    match (exit_code, verdict.status) {
        (Some(SELFTEST_PASSED), VerdictStatus::Pass) => Ok(()),
        (Some(SELFTEST_FAILED), VerdictStatus::Fail) => {
            // The names go in the error, not only in the log, because a caller
            // deciding whether the *right* thing broke cannot read the log.
            bail!("{SELFTEST_FAILURE}: {}", observation.failed.join("; "))
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
fn report_hang(port: u16) {
    let Ok(state) = monitor(port, "info registers\n") else {
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
    let Ok(code) = monitor(port, &format!("x/8i 0x{rip}\n")) else {
        return;
    };
    for line in code
        .lines()
        .filter(|line| line.trim_start().starts_with("0x"))
    {
        println!("[xtask] hung in:  {}", line.trim());
    }
}

/// One monitor exchange on a connection of its own.
///
/// A fresh connection per command because the monitor serves one at a time,
/// and every read is bounded: this runs when the machine is already wedged.
fn monitor(port: u16, command: &str) -> Result<String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, MONITOR_TIMEOUT)
        .context("connecting to the QEMU monitor")?;
    stream
        .set_read_timeout(Some(MONITOR_TIMEOUT))
        .context("setting the QEMU monitor read timeout")?;
    stream
        .write_all(command.as_bytes())
        .context("writing a QEMU monitor command")?;

    let mut text = String::new();
    let mut buffer = [0u8; 4096];
    loop {
        // The prompt appears once on connect and once after the reply.
        if text.matches(MONITOR_PROMPT).count() >= 2 {
            break;
        }
        let read = stream
            .read(&mut buffer)
            .context("reading a QEMU monitor reply")?;
        if read == 0 {
            bail!("QEMU monitor closed before completing its reply");
        }
        text.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    Ok(text)
}

/// Long enough for a reply, short enough not to extend a failing run.
const MONITOR_TIMEOUT: Duration = Duration::from_millis(1500);

/// What the monitor prints when it wants a command.
const MONITOR_PROMPT: &str = "(qemu)";

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
    let qemu_dir = locate()?;
    let code = firmware(&qemu_dir, CODE_FIRMWARE)?;
    let vars_template = firmware(&qemu_dir, VARS_FIRMWARE)?;
    let vars = paths.efi_vars();
    std::fs::copy(&vars_template, &vars)
        .with_context(|| format!("copying {} to {}", vars_template.display(), vars.display()))?;

    let listener =
        TcpListener::bind(("127.0.0.1", 0)).context("binding the QMP loopback listener")?;
    listener
        .set_nonblocking(true)
        .context("making the QMP listener non-blocking")?;
    let qmp_endpoint = format!(
        "tcp:{}",
        listener.local_addr().context("reading the QMP port")?
    );

    // Bound and dropped: QEMU wants a port number, and this is how a free
    // one is found without guessing.
    let monitor_port = {
        let probe = TcpListener::bind(("127.0.0.1", 0)).context("probing for a monitor port")?;
        probe
            .local_addr()
            .context("reading the monitor port")?
            .port()
    };

    let emulator = qemu_dir.join(exe_name("qemu-system-x86_64"));
    println!("[xtask] booting the self-test kernel");

    let mut child = Command::new(&emulator)
        .args(MACHINE)
        .args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .args(["-qmp", &qmp_endpoint])
        .args([
            "-monitor",
            &format!("tcp:127.0.0.1:{monitor_port},server,nowait"),
        ])
        .args(drive(&format!(
            "if=pflash,format=raw,readonly=on,file={}",
            code.display()
        )))
        .args(drive(&format!(
            "if=pflash,format=raw,file={}",
            vars.display()
        )))
        .args(drive(&format!("format=raw,file={}", image.display())))
        .args(storage(paths.root())?)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", emulator.display()))?;

    let stdout = child.stdout.take().context("taking QEMU's stdout pipe")?;
    let marker_seen = Arc::new(AtomicBool::new(false));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let forwarder = forward_serial(
        stdout,
        Arc::clone(&marker_seen),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
    );
    inject_on_marker(listener, marker_seen);

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for QEMU")? {
            break status;
        }
        // A panicking kernel halts rather than reporting, so the timeout
        // would otherwise be the only way it ever ends.
        if panicked.load(Ordering::Acquire) {
            let _ = child.kill();
            finish_selftest_serial(forwarder)?;
            bail!(PANIC_FAILURE);
        }
        if serial_failed.load(Ordering::Acquire) {
            let _ = child.kill();
            finish_selftest_serial(forwarder)?;
            unreachable!("a flagged serial failure returned success");
        }
        if Instant::now() >= deadline {
            report_hang(monitor_port);
            let _ = child.kill();
            finish_selftest_serial(forwarder)?;
            bail!(
                "self-test did not finish within {} seconds",
                timeout.as_secs()
            );
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    // Trailing serial lines print before the verdict does.
    let observation = finish_selftest_serial(forwarder)?;

    correlate_selftest(status.code(), observation)?;
    println!("[xtask] self-test passed");
    Ok(())
}

/// Forward every serial line to the console, flagging the ready marker.
///
/// A kernel that dies mid-line still shows its last words: the unterminated
/// remainder at end-of-stream is forwarded too.
fn forward_serial(
    stdout: ChildStdout,
    marker_seen: Arc<AtomicBool>,
    panicked: Arc<AtomicBool>,
    serial_failed: Arc<AtomicBool>,
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
            // One print per line, so a message from another thread cannot
            // land between a dying kernel's last words and their newline.
            let text = String::from_utf8_lossy(&line);
            if text.ends_with('\n') {
                print!("{text}");
            } else {
                println!("{text}");
            }
            let _ = std::io::stdout().flush();
            if text.contains(READY_MARKER) {
                marker_seen.store(true, Ordering::Release);
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
/// QMP stall — a slow handshake, a peer dribbling bytes — can ever delay the
/// loop that enforces the run's deadline. The thread is deliberately never
/// joined: if QEMU dies without connecting, the accept loop below would spin
/// until process exit reaps it, and every exit path already has the verdict
/// by then.
fn inject_on_marker(listener: TcpListener, marker_seen: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let outcome = accept(&listener).and_then(|mut qmp| {
            while !marker_seen.load(Ordering::Acquire) {
                std::thread::sleep(POLL_INTERVAL);
            }
            INJECTED_QCODES
                .iter()
                .try_for_each(|qcode| qmp.inject_key(qcode))
        });
        match outcome {
            Ok(()) => println!("[xtask] qmp: {} keystrokes injected", INJECTED_QCODES.len()),
            Err(reason) => {
                println!("[xtask] qmp: {reason:#}; the keystroke check will report it");
            }
        }
    });
}

/// Wait for QEMU's connection and negotiate on it.
///
/// The listener stays non-blocking — `run_selftest` configured it before the
/// spawn — so this polls rather than parks, and a QEMU that dies before
/// connecting leaves a loop that process exit cleans up.
fn accept(listener: &TcpListener) -> Result<Qmp> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Qmp::handshake(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(error) => return Err(error).context("accepting QEMU's QMP connection"),
        }
    }
}

fn accept_before(listener: &TcpListener, deadline: Instant) -> Result<Qmp> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Qmp::handshake(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    bail!("QEMU did not open its QMP connection");
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

fn keyboard_irq_count(port: u16) -> Result<u64> {
    let report = monitor(port, "info irq\n").context("querying keyboard IRQ progress")?;
    irq_count(&report, 1).context("QEMU monitor returned no IRQ statistics")
}

fn await_keyboard_irq(
    child: &mut Child,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
    monitor_port: u16,
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
        let after = keyboard_irq_count(monitor_port)?;
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
    let qemu_dir = locate()?;
    let code = firmware(&qemu_dir, CODE_FIRMWARE)?;
    let vars_template = firmware(&qemu_dir, VARS_FIRMWARE)?;
    let vars = paths.efi_vars();
    std::fs::copy(&vars_template, &vars)
        .with_context(|| format!("copying {} to {}", vars_template.display(), vars.display()))?;

    let listener =
        TcpListener::bind(("127.0.0.1", 0)).context("binding the shipped QMP listener")?;
    listener
        .set_nonblocking(true)
        .context("making the shipped QMP listener non-blocking")?;
    let qmp_endpoint = format!(
        "tcp:{}",
        listener.local_addr().context("reading the QMP port")?
    );
    let monitor_port = {
        let probe = TcpListener::bind(("127.0.0.1", 0)).context("probing for a monitor port")?;
        probe
            .local_addr()
            .context("reading the monitor port")?
            .port()
    };

    let emulator = qemu_dir.join(exe_name("qemu-system-x86_64"));
    println!("[xtask] booting the shipped kernel");

    let mut child = Command::new(&emulator)
        .args(MACHINE)
        .args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(["-qmp", &qmp_endpoint])
        .args([
            "-monitor",
            &format!("tcp:127.0.0.1:{monitor_port},server,nowait"),
        ])
        .args(drive(&format!(
            "if=pflash,format=raw,readonly=on,file={}",
            code.display()
        )))
        .args(drive(&format!(
            "if=pflash,format=raw,file={}",
            vars.display()
        )))
        .args(drive(&format!("format=raw,file={}", image.display())))
        .args(storage(paths.root())?)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", emulator.display()))?;

    let stdout = child.stdout.take().context("taking QEMU's stdout pipe")?;
    let settled = Arc::new(AtomicBool::new(false));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let forwarder = watch_serial(
        stdout,
        Arc::clone(&settled),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        None,
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
        let before = keyboard_irq_count(monitor_port)?;
        let mut qmp = accept_before(&listener, Instant::now() + SHIPPED_CHALLENGE_TIMEOUT)?;
        qmp.inject_key(SHIPPED_CHALLENGE_QCODE)
            .context("injecting the shipped-kernel liveness key")?;
        let after =
            await_keyboard_irq(&mut child, &panicked, &serial_failed, monitor_port, before)?;
        println!("[xtask] shipped liveness: keyboard IRQ count {before} -> {after}");
        Ok(())
    })
    .and_then(|()| watch(&mut child, &panicked, &serial_failed, settle, None));

    let _ = child.kill();
    let serial_outcome = finish_boot_serial(forwarder);

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
    println!("[xtask] shipped kernel boots, responds, and settles");
    Ok(())
}

/// How long to allow for a cold start before the marker must have appeared.
const BOOT_TIMEOUT: Duration = Duration::from_secs(45);

/// A processor with no x2APIC, and none of the protection bits `-cpu max` has.
///
/// The last `-cpu` wins, so this overrides [`MACHINE`]'s.
const DEGRADED_CPU: [&str; 2] = ["-cpu", "qemu64,-x2apic"];

/// What the kernel must say when it finds no x2APIC.
///
/// The refusal D-33 chose and D-90 made a gate check: xAPIC needs page tables
/// that do not exist when the APIC is brought up, so there is no fallback —
/// only a diagnosis.
const NO_X2APIC_MARKER: &str = "no x2apic mode";

/// Boot on a machine the kernel cannot fully use, and require it to say so.
///
/// The negative half of D-32 and D-33: every other boot in the gate runs on
/// `-cpu max`, where every feature is present, so nothing exercises what the
/// kernel does when one is missing. A precondition that is only ever met is
/// indistinguishable from one that is never checked (D-90).
///
/// The bar is deliberately not "it works": it is that the kernel *reports* the
/// missing feature and still reaches its steady state, rather than
/// triple-faulting on a register that is not there.
pub fn run_degraded_check(paths: &Paths, image: &Path, settle: Duration) -> Result<()> {
    let qemu_dir = locate()?;
    let code = firmware(&qemu_dir, CODE_FIRMWARE)?;
    let vars_template = firmware(&qemu_dir, VARS_FIRMWARE)?;
    let vars = paths.efi_vars();
    std::fs::copy(&vars_template, &vars)
        .with_context(|| format!("copying {} to {}", vars_template.display(), vars.display()))?;

    let emulator = qemu_dir.join(exe_name("qemu-system-x86_64"));
    println!("[xtask] booting a kernel with no x2apic");

    let mut child = Command::new(&emulator)
        .args(MACHINE)
        .args(DEGRADED_CPU)
        .args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(drive(&format!(
            "if=pflash,format=raw,readonly=on,file={}",
            code.display()
        )))
        .args(drive(&format!(
            "if=pflash,format=raw,file={}",
            vars.display()
        )))
        .args(drive(&format!("format=raw,file={}", image.display())))
        .args(storage(paths.root())?)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", emulator.display()))?;

    let stdout = child.stdout.take().context("taking QEMU's stdout pipe")?;
    let settled = Arc::new(AtomicBool::new(false));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let refused = Arc::new(AtomicBool::new(false));
    let forwarder = watch_serial(
        stdout,
        Arc::clone(&settled),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        Some((NO_X2APIC_MARKER, Arc::clone(&refused))),
    );

    let outcome = watch(
        &mut child,
        &panicked,
        &serial_failed,
        BOOT_TIMEOUT,
        Some(&settled),
    )
    .and_then(|()| watch(&mut child, &panicked, &serial_failed, settle, None));

    let _ = child.kill();
    let serial_outcome = finish_boot_serial(forwarder);

    serial_outcome?;
    outcome?;
    if !refused.load(Ordering::Acquire) {
        bail!(
            "the kernel settled without reporting {NO_X2APIC_MARKER:?}: the x2apic precondition is not being checked"
        );
    }
    println!("[xtask] degraded kernel reports the missing x2apic and settles");
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

/// Forward every serial line, flagging the two the boot check cares about.
fn watch_serial(
    stdout: ChildStdout,
    settled: Arc<AtomicBool>,
    panicked: Arc<AtomicBool>,
    serial_failed: Arc<AtomicBool>,
    extra: Option<(&'static str, Arc<AtomicBool>)>,
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
            if let Some((marker, seen)) = extra.as_ref()
                && text.contains(marker)
            {
                seen.store(true, Ordering::Release);
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
    fn drive_produces_a_flag_and_its_value() {
        assert_eq!(
            drive("format=raw,file=disk.img"),
            ["-drive", "format=raw,file=disk.img"]
        );
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
        };
        assert!(correlate_selftest(Some(SELFTEST_PASSED), observation).is_ok());

        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(passing_verdict()),
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
            selftest.contains(READY_MARKER),
            "the kernel does not print {READY_MARKER:?}"
        );
        assert!(
            main.contains(SETTLED_MARKER),
            "the kernel does not print {SETTLED_MARKER:?}"
        );
        assert!(
            panic.contains(PANIC_MARKER),
            "the kernel does not print {PANIC_MARKER:?}"
        );
    }

    /// The C header and the kernel agree about the ABI they both describe.
    ///
    /// `userland/abi/wizard_abi.h` is the native ABI written a second time, for
    /// a consumer that cannot read Rust. Two transcriptions of one table is how
    /// `x86-tables` and `trap-cause` came to disagree about vector 28, and the
    /// answer there was a test that compares them — this is the same test, one
    /// language further out (D-105, D-114).
    ///
    /// Only the numbers a program would get *wrong* are compared: an address it
    /// reads the ring at, a version it checks, the opcodes it submits and the
    /// statuses it decodes. A drift in any of those is a program that compiles,
    /// runs, and reads the wrong page.
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

    /// Every sysdep's tag agrees with the README's tally.
    ///
    /// The three tags — MAPS, OWED, ABSENT — are the whole finding of D-74's
    /// slice: which POSIX calls this ABI already has the shape for, which it
    /// owes a door, and which it will never have. The counts live in the
    /// README and the tags live in the sysdeps, and the two disagreed in two
    /// places: `sys_libc_log` called itself MAPS where the README counts only
    /// its *shape*, and `sys_open` called itself OWED where the README has
    /// always said ABSENT — which is the distinction the taxonomy exists to
    /// draw (D-124).
    #[test]
    fn every_sysdep_tag_agrees_with_the_readme() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let sysdeps = std::fs::read_to_string(root.join("userland/sysdeps/wizard/generic.cpp"))
            .expect("generic.cpp");
        let readme = std::fs::read_to_string(root.join("userland/README.md")).expect("README.md");

        // One tag line per sysdep: `// sys_name. TAG`, which is the register
        // the file is written in throughout.
        let mut tags: Vec<(&str, &str)> = Vec::new();
        for line in sysdeps.lines() {
            let Some(rest) = line.trim().strip_prefix("// sys_") else {
                continue;
            };
            let Some((name, after)) = rest.split_once(". ") else {
                continue;
            };
            for tag in ["MAPS", "OWED", "ABSENT"] {
                if after.starts_with(tag) {
                    tags.push((name, tag));
                    break;
                }
            }
        }

        // The four that reach the kernel through the bare gate, plus the
        // file-descriptor five, plus memory, threads and futexes.
        assert_eq!(
            tags.len(),
            16,
            "mlibc's sixteen mandatory tags, one tag line each; found {tags:?}"
        );

        for (tag, expected) in [("MAPS", 5), ("OWED", 10), ("ABSENT", 1)] {
            let counted = tags.iter().filter(|(_, found)| *found == tag).count();
            // The README states it as a table row.
            let declared = readme
                .lines()
                .find_map(|line| {
                    let mut cells = line.split('|').map(str::trim);
                    (cells.next()?.is_empty() && cells.next()? == format!("**{tag}**"))
                        .then(|| cells.next()?.parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_else(|| panic!("no {tag} row in the README's table"));

            assert_eq!(counted, expected, "{tag}: the sysdeps tag {counted}");
            assert_eq!(declared, expected, "{tag}: the README says {declared}");
        }

        // And `sys_open` is the ABSENT one, which is the disagreement that
        // made this test worth writing.
        assert_eq!(
            tags.iter()
                .find(|(_, tag)| *tag == "ABSENT")
                .map(|(name, _)| *name),
            Some("open"),
            "`sys_open` is the one the native ABI has nothing of the kind for"
        );
    }

    #[test]
    fn the_c_header_and_the_kernel_agree_about_the_abi() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let header = std::fs::read_to_string(root.join("userland/abi/wizard_abi.h"))
            .expect("userland/abi/wizard_abi.h");
        let queue = std::fs::read_to_string(root.join("kernel/src/queue.rs")).expect("queue.rs");
        let user = std::fs::read_to_string(root.join("kernel/src/user.rs")).expect("user.rs");
        let proto =
            std::fs::read_to_string(root.join("libs/ring-proto/src/lib.rs")).expect("ring-proto");

        // `#define NAME value`, with the value taken verbatim up to any comment.
        let defined = |name: &str| -> String {
            header
                .lines()
                .find_map(|line| line.trim().strip_prefix(&format!("#define {name} ")))
                .map(|value| {
                    value
                        .split("/*")
                        .next()
                        .unwrap_or(value)
                        .trim()
                        .trim_matches(|c| c == '(' || c == ')')
                        .to_string()
                })
                .unwrap_or_else(|| panic!("no `#define {name}` in wizard_abi.h"))
        };
        // A `pub(crate) const NAME: TYPE = value;` from a Rust source.
        let rust = |source: &str, name: &str| -> String {
            source
                .lines()
                .find_map(|line| {
                    let line = line.trim();
                    let rest = line
                        .strip_prefix(&format!("pub(crate) const {name}: "))
                        .or_else(|| line.strip_prefix(&format!("pub const {name}: ")))?;
                    rest.split_once(" = ")
                        .map(|(_, value)| value.trim_end_matches(';').replace('_', ""))
                })
                .unwrap_or_else(|| panic!("no `{name}` in the kernel source"))
        };

        // The ring's address. The one number that, if it drifted, would have a
        // program reading a page nobody wrote.
        assert!(
            defined("WIZ_RING_ADDRESS").contains(&rust(&user, "RING").replace("0x", "")),
            "the ring address drifted: header {:?}, kernel {:?}",
            defined("WIZ_RING_ADDRESS"),
            rust(&user, "RING")
        );

        // The protocol version, which is the field a program checks first.
        assert_eq!(
            defined("WIZ_RING_VERSION").trim_end_matches('u'),
            rust(&proto, "VERSION"),
            "the ring version drifted"
        );

        // The opcodes and statuses, which the kernel writes as match arms
        // rather than as constants — so they are searched for as the arms.
        assert!(
            queue.contains("1 => Self::Echo"),
            "WIZ_OP_ECHO is not opcode 1 in the kernel"
        );
        assert_eq!(defined("WIZ_OP_ECHO").trim_end_matches('u'), "1");
        assert!(
            queue.contains("2 => Self::Uptime"),
            "WIZ_OP_UPTIME is not opcode 2 in the kernel"
        );
        assert_eq!(defined("WIZ_OP_UPTIME").trim_end_matches('u'), "2");
        for (name, number, arm) in [
            ("WIZ_OP_DUPLICATE", "3", "3 => Self::Duplicate"),
            ("WIZ_OP_TIMER", "4", "4 => Self::Timer"),
            ("WIZ_OP_CANCEL", "5", "5 => Self::Cancel"),
        ] {
            assert!(queue.contains(arm), "the kernel does not decode `{arm}`");
            assert_eq!(
                defined(name).trim_end_matches('u'),
                number,
                "{name} drifted"
            );
        }

        assert_eq!(
            defined("WIZ_STATUS_OK").trim_end_matches('u'),
            rust(&queue, "STATUS_OK")
        );
        assert_eq!(
            defined("WIZ_STATUS_UNKNOWN_OPCODE").trim_end_matches('u'),
            rust(&queue, "STATUS_UNKNOWN_OPCODE")
        );
        for (name, konstant) in [
            ("WIZ_STATUS_NO_HANDLE_SLOT", "STATUS_NO_HANDLE_SLOT"),
            ("WIZ_STATUS_BAD_HANDLE", "STATUS_BAD_HANDLE"),
            ("WIZ_STATUS_DENIED", "STATUS_DENIED"),
            ("WIZ_STATUS_CANCELLED", "STATUS_CANCELLED"),
            ("WIZ_STATUS_NOT_FOUND", "STATUS_NOT_FOUND"),
            ("WIZ_STATUS_NO_ROOM", "STATUS_NO_ROOM"),
        ] {
            assert_eq!(
                defined(name).trim_end_matches('u'),
                rust(&queue, konstant),
                "{name} drifted from the kernel"
            );
        }

        // The time page. Its address is the one number that, if it drifted,
        // would have a program reading the clock out of an unmapped page — and
        // its layout is a second transcription of `timepage.rs`'s offsets,
        // which is exactly the arrangement this test exists for (D-166).
        let timepage =
            std::fs::read_to_string(root.join("kernel/src/timepage.rs")).expect("timepage.rs");
        assert_eq!(
            defined("WIZ_TIME_ADDRESS")
                .replace("uintptr_t)", "")
                .replace("ull", ""),
            rust(&timepage, "TIME"),
            "the time page's address is not the kernel's"
        );
        assert_eq!(
            defined("WIZ_TIME_VERSION").trim_end_matches('u'),
            rust(&timepage, "VERSION")
        );
        assert_eq!(
            defined("WIZ_TIME_MODE_NONE").trim_end_matches('u'),
            rust(&timepage, "MODE_NONE")
        );
        assert_eq!(
            defined("WIZ_TIME_MODE_COUNTER").trim_end_matches('u'),
            rust(&timepage, "MODE_COUNTER")
        );
        // The clock region's offset, which the header states as a number and
        // the kernel as a private constant.
        let clock_at = timepage
            .lines()
            .find_map(|line| line.trim().strip_prefix("const CLOCK_AT: usize = "))
            .map(|value| value.trim_end_matches(';').to_string())
            .expect("no CLOCK_AT in timepage.rs");
        assert_eq!(defined("WIZ_TIME_CLOCK_AT").trim_end_matches('u'), clock_at);

        // And the field offsets, which are the part a program gets wrong
        // silently: a `mode` read from the wrong place is a clock that reads as
        // revoked, or worse, as usable when it is not.
        for (field, offset) in [
            ("SEQ", 0usize),
            ("MODE", 4),
            ("BASE_NS", 8),
            ("FRACTION", 16),
            ("LAST_TICKS", 24),
            ("MULTIPLIER", 32),
            ("SHIFT", 40),
        ] {
            let declared = timepage
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix(&format!("pub(super) const {field}: usize = "))
                })
                .map(|value| value.trim_end_matches(';').to_string())
                .unwrap_or_else(|| panic!("no clock::{field} in timepage.rs"));
            assert_eq!(declared, offset.to_string(), "clock::{field} moved");
        }

        // And the gate the calls go through — read from `vector.rs`, not from
        // a literal. Comparing the header to `"0x80"` never opened the kernel
        // at all, so `vector::SYSCALL` could move to `0x81` with every syscall
        // in the header wrong and the check green (D-124).
        let vectors = std::fs::read_to_string(root.join("kernel/src/arch/x86_64/vector.rs"))
            .expect("vector.rs");
        let syscall = vectors
            .lines()
            .find_map(|line| line.trim().strip_prefix("pub(crate) const SYSCALL: u8 = "))
            .map(|value| value.trim_end_matches(';').to_string())
            .expect("no SYSCALL in vector.rs");
        assert_eq!(defined("WIZ_SYSCALL_VECTOR"), syscall);
        // The program's own `int 0x80` is a fourth transcription of it.
        let program =
            std::fs::read_to_string(root.join("xtask/src/image/program.rs")).expect("program.rs");
        assert!(
            program.contains("0xCD, 0x80"),
            "the emitted program does not call {syscall}"
        );

        // The ring's address is compared for equality, not containment: a
        // header saying `0x10000200000000000ull` used to pass (D-124).
        assert_eq!(
            defined("WIZ_RING_ADDRESS")
                .replace("uintptr_t)", "")
                .replace("ull", ""),
            rust(&user, "RING"),
            "the ring address is not the kernel's"
        );

        // Every call number, against the kernel's own decoder — the whole
        // enum was invisible to `defined`, which parses only `#define`, and
        // D-118 renumbered it.
        for (name, number) in [
            ("WIZ_CALL_EXIT", "0"),
            ("WIZ_CALL_RING_ENTER", "1"),
            ("WIZ_CALL_TCB_SET", "2"),
            ("WIZ_CALL_RING_WAIT", "3"),
        ] {
            let declared = header
                .lines()
                .find_map(|line| line.trim().strip_prefix(&format!("{name} = ")))
                .map(|value| value.split(',').next().unwrap_or(value).trim().to_string())
                .unwrap_or_else(|| panic!("no `{name}` in wizard_abi.h"));
            assert_eq!(declared, number, "{name} drifted from the header");
        }
        let calls = std::fs::read_to_string(root.join("kernel/src/user.rs")).expect("user.rs");
        for arm in [
            "0 => Self::Exit",
            "1 => Self::RingEnter",
            "2 => Self::ThreadBase",
            "3 => Self::RingWait",
        ] {
            assert!(calls.contains(arm), "the kernel does not decode `{arm}`");
        }

        // And nothing new slipped in unaccounted for: every `WIZ_*` name the
        // header defines is one this test knows about. Without it the header
        // grows constants nobody compares, which is how nineteen of
        // twenty-six came to be unchecked (D-124).
        let known = [
            "WIZ_HANDLE_INVALID",
            "WIZ_RIGHT_DUPLICATE",
            "WIZ_RIGHT_TRANSFER",
            "WIZ_RIGHT_WAIT",
            "WIZ_RIGHT_INSPECT",
            "WIZ_RIGHT_READ",
            "WIZ_RIGHT_WRITE",
            "WIZ_RIGHT_EXECUTE",
            "WIZ_RIGHT_MAP",
            "WIZ_RING_ADDRESS",
            "WIZ_RING_VERSION",
            "WIZ_RING_ENTRIES_AT",
            "WIZ_OP_ECHO",
            "WIZ_OP_UPTIME",
            "WIZ_OP_DUPLICATE",
            "WIZ_OP_TIMER",
            "WIZ_OP_CANCEL",
            "WIZ_OP_SIGNAL",
            "WIZ_STATUS_OK",
            "WIZ_STATUS_UNKNOWN_OPCODE",
            "WIZ_STATUS_NO_HANDLE_SLOT",
            "WIZ_STATUS_BAD_HANDLE",
            "WIZ_STATUS_DENIED",
            "WIZ_STATUS_CANCELLED",
            "WIZ_STATUS_NOT_FOUND",
            "WIZ_STATUS_NO_ROOM",
            "WIZ_TIME_ADDRESS",
            "WIZ_TIME_VERSION",
            "WIZ_TIME_CLOCK_AT",
            "WIZ_TIME_MODE_NONE",
            "WIZ_TIME_MODE_COUNTER",
            "WIZ_SYSCALL_VECTOR",
            "WIZ_USER_END",
            "WIZ_ENOSYS",
            "WIZ_EINVAL",
            "WIZ_EBADF",
            "WIZ_EPERM",
            "WIZ_EAGAIN",
            "WIZ_ENOMEM",
        ];
        for line in header.lines() {
            if let Some(rest) = line.trim().strip_prefix("#define WIZ_") {
                let name = format!("WIZ_{}", rest.split_whitespace().next().unwrap_or(""));
                assert!(
                    known.contains(&name.as_str()),
                    "{name} is defined and nothing accounts for it"
                );
            }
        }

        // The rights, against `libs/handle-space`.
        let rights = std::fs::read_to_string(root.join("libs/handle-space/src/lib.rs"))
            .expect("handle-space");
        for (name, shift) in [
            ("WIZ_RIGHT_DUPLICATE", "DUPLICATE"),
            ("WIZ_RIGHT_TRANSFER", "TRANSFER"),
            ("WIZ_RIGHT_WAIT", "WAIT"),
            ("WIZ_RIGHT_INSPECT", "INSPECT"),
            ("WIZ_RIGHT_READ", "READ"),
            ("WIZ_RIGHT_WRITE", "WRITE"),
            ("WIZ_RIGHT_EXECUTE", "EXECUTE"),
            ("WIZ_RIGHT_MAP", "MAP"),
        ] {
            let bit = rights
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix(&format!("pub const {shift}: Self = Self(1 << "))
                })
                .map(|value| value.trim_end_matches(");").to_string())
                .unwrap_or_else(|| panic!("no `{shift}` in handle-space"));
            let declared = defined(name);
            assert!(
                declared.contains(&format!("1u << {bit}")),
                "{name} is {declared:?}, not bit {bit}"
            );
        }

        // And the user boundary, read from the one place that defines it.
        // `usercopy` re-exports this rather than transcribing it (D-124), so
        // the assertion follows the definition rather than the re-export —
        // and the re-export is checked too, because a second literal
        // reappearing there is the thing that was fixed.
        let paging = std::fs::read_to_string(root.join("wost/src/arch/x86_64/paging.rs"))
            .expect("paging.rs");
        assert!(
            paging.contains("pub const USER_END: u64 = 0x0000_8000_0000_0000;"),
            "the kernel's user boundary moved"
        );
        let usercopy = std::fs::read_to_string(root.join("wost/src/arch/x86_64/usercopy.rs"))
            .expect("usercopy.rs");
        assert!(
            usercopy.contains("pub use super::paging::USER_END;"),
            "usercopy states the boundary again instead of re-exporting it"
        );
        assert!(
            defined("WIZ_USER_END").contains("0x0000800000000000"),
            "the header's user boundary is not the kernel's"
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
