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
const MACHINE: &[&str] = &["-machine", "q35", "-cpu", "max", "-m", "512M"];

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

/// QEMU exits with `(value << 1) | 1`, so the kernel's 0x10 arrives as 33 and
/// its 0x11 as 35. Kept in step with `kernel/src/arch/x86_64/debug_exit.rs`.
const SELFTEST_PASSED: i32 = 33;
const SELFTEST_FAILED: i32 = 35;

/// The serial verdict contract shared with `kernel/src/selftest.rs`.
const SELFTEST_VERDICT_PREFIX: &str = "selftest: verdict ";
const SELFTEST_VERDICT_VERSION: &str = "v1";
const EXPECTED_SELFTEST_CHECKS: usize = 80;

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
            bail!("self-test reported failures - see the log above")
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
            bail!("the self-test kernel panicked — see the log above");
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
    println!("[xtask] shipped kernel boots, responds, and settles");
    Ok(())
}

/// How long to allow for a cold start before the marker must have appeared.
const BOOT_TIMEOUT: Duration = Duration::from_secs(45);

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
            verdict: Some(passing_verdict()),
        };
        assert!(correlate_selftest(Some(SELFTEST_PASSED), observation).is_ok());

        let observation = SerialObservation {
            verdict: Some(passing_verdict()),
        };
        let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
        assert!(error.to_string().contains("debug-exit"));
    }

    #[test]
    fn zero_or_skipped_checks_cannot_pass() {
        for passed in [0, EXPECTED_SELFTEST_CHECKS - 1] {
            let observation = SerialObservation {
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
}
