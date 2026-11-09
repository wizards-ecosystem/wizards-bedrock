use std::io::Read;

use super::*;

/// QEMU exits with `(value << 1) | 1`, so the kernel's 0x10 arrives as 33 and
/// its 0x11 as 35. Kept in step with `kernel/src/arch/x86_64/debug_exit.rs`.
pub(super) const SELFTEST_PASSED: i32 = 33;
pub(super) const SELFTEST_FAILED: i32 = 35;

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
        /// The complete single-line panic body.
        transcript: String,
        /// Named failures printed before the terminal panic.
        failed: Vec<String>,
    },
}

/// A complete degraded-boot outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DegradedOutcome {
    /// Every required refusal and fallback marker preceded a settled boot.
    Settled,
    /// The kernel stopped before settling.
    Panicked {
        /// The complete single-line panic body.
        transcript: String,
        /// The no-x2APIC refusal appeared before the panic.
        no_x2apic_diagnosed: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VerdictStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SelftestVerdict {
    pub(super) status: VerdictStatus,
    pub(super) passed: usize,
    pub(super) failed: usize,
    pub(super) expected: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StackSample {
    pub(super) used: u64,
    pub(super) capacity: u64,
    pub(super) headroom: u64,
    pub(super) leaf: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SerialObservation {
    pub(super) verdict: Option<SelftestVerdict>,
    pub(super) baseline: Option<PerformanceSample>,
    pub(super) nvme_stack: Option<StackSample>,
    pub(super) qemu_tcg_isolation: bool,
    pub(super) strict_dma_handoff: bool,
    /// Ordered failures, so mutation grading cannot accept an unrelated red
    /// (D-134).
    pub(super) failed: Vec<String>,
    pub(super) panic: Option<String>,
    pub(super) panic_complete: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BootSerialObservation {
    pub(super) panic: Option<String>,
    pub(super) panic_complete: bool,
    pub(super) no_x2apic_before_panic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanicTransition {
    Unchanged,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LogSeverity {
    Info,
    Warning,
    Error,
}

impl LogSeverity {
    pub(super) const fn prefix(self) -> &'static str {
        match self {
            Self::Info => "[info ] ",
            Self::Warning => "[warn ] ",
            Self::Error => "[error] ",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RequiredLogRecord {
    pub(super) severity: LogSeverity,
    pub(super) message: &'static str,
}

impl RequiredLogRecord {
    pub(super) const fn info(message: &'static str) -> Self {
        Self {
            severity: LogSeverity::Info,
            message,
        }
    }

    pub(super) const fn warning(message: &'static str) -> Self {
        Self {
            severity: LogSeverity::Warning,
            message,
        }
    }
}

impl PanicTransition {
    pub(super) fn publish(self, panicked: &AtomicBool) {
        if self == Self::Completed {
            panicked.store(true, Ordering::Release);
        }
    }
}

pub(super) fn is_complete_panic_record(line: &str) -> bool {
    terminated_serial_text(line).and_then(|line| line.strip_prefix("[error] "))
        == Some(COMPLETE_MARKER)
}

fn terminated_serial_text(line: &str) -> Option<&str> {
    let line = line.strip_suffix('\n')?;
    let text = line.strip_suffix('\r').unwrap_or(line);
    if text.contains('\r') || text.contains('\n') {
        return None;
    }
    Some(text)
}

fn observe_panic_line(
    panic: &mut Option<String>,
    panic_complete: &mut bool,
    line: &str,
) -> Result<Option<PanicTransition>> {
    if is_complete_panic_record(line) {
        if panic.is_none() {
            bail!("the kernel completed a panic record it never started");
        }
        if *panic_complete {
            bail!("the kernel completed its panic record more than once");
        }
        *panic_complete = true;
        return Ok(Some(PanicTransition::Completed));
    }
    if *panic_complete {
        bail!("the kernel printed serial data after its terminal panic record");
    }
    if panic.is_some() && !*panic_complete {
        bail!("the kernel continued a single-line panic record");
    }
    if line.starts_with(OPEN_PREFIX) {
        let text = terminated_serial_text(line)
            .context("the kernel printed an unterminated panic opening record")?;
        *panic = Some(text.to_string());
        return Ok(Some(PanicTransition::Unchanged));
    }
    Ok(None)
}

pub(super) fn completed_panic(
    panic: Option<String>,
    panic_complete: bool,
) -> Result<Option<String>> {
    match (panic, panic_complete) {
        (None, false) => Ok(None),
        (Some(transcript), true) => Ok(Some(transcript)),
        (Some(_), false) => bail!("QEMU stopped with an incomplete serial panic record"),
        (None, true) => bail!("QEMU observed a panic completion without its opening record"),
    }
}

fn is_exact_no_x2apic_diagnosis(line: &str) -> bool {
    is_exact_log_message(line, LogSeverity::Warning, NO_X2APIC_DIAGNOSIS)
}

pub(super) fn observe_boot_line(
    observation: &mut BootSerialObservation,
    line: &str,
) -> Result<PanicTransition> {
    if let Some(transition) = observe_panic_line(
        &mut observation.panic,
        &mut observation.panic_complete,
        line,
    )? {
        return Ok(transition);
    }
    if is_exact_no_x2apic_diagnosis(line) {
        observation.no_x2apic_before_panic = true;
    }
    Ok(PanicTransition::Unchanged)
}

pub(super) fn parse_failed_check(line: &str) -> Option<&str> {
    let message = log_message(line, LogSeverity::Error)?;
    let name = message
        .strip_prefix(SELFTEST_CHECK_PREFIX)?
        .strip_suffix(SELFTEST_CHECK_FAILED)?;
    (!name.is_empty()).then_some(name)
}

pub(super) fn is_exact_log_message(line: &str, severity: LogSeverity, expected: &str) -> bool {
    log_message(line, severity) == Some(expected)
}

fn log_message(line: &str, severity: LogSeverity) -> Option<&str> {
    terminated_serial_text(line).and_then(|line| line.strip_prefix(severity.prefix()))
}

pub(super) fn is_required_boot_marker(line: &str, record: RequiredLogRecord) -> bool {
    if record == NO_X2APIC_RECORD {
        return is_exact_no_x2apic_diagnosis(line);
    }
    is_exact_log_message(line, record.severity, record.message)
}

pub(super) fn is_settled_record(line: &str) -> bool {
    let Some(message) = log_message(line, LogSeverity::Info) else {
        return false;
    };
    message
        .strip_prefix(SETTLED_MARKER)
        .and_then(|uptime| uptime.strip_suffix(" ms"))
        .and_then(canonical_decimal)
        .is_some()
}

// Cover ordinary input and both editing paths (D-51).
pub(super) const INJECTED_QCODES: &[&str] = &["a", "backspace", "ret"];

pub(super) const STORAGE_DELAY_QCODES: &[&str] = &["c"];

// A distinct second key prevents a queued first-round key satisfying the
// translated-path check (D-155, D-179).
pub(super) const TRANSLATED_QCODES: &[&str] = &["b"];

pub(super) const INTERACTIVE_MARKERS: &[RequiredLogRecord] = &[
    RequiredLogRecord::info(STORAGE_DELAY_MARKER),
    RequiredLogRecord::info(READY_MARKER),
    RequiredLogRecord::info(TRANSLATED_MARKER),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InteractiveMarkerOrder {
    Strict,
    Mutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InjectionAction {
    Wait,
    Inject,
    Skip,
}

pub(super) const fn injection_action(observed: usize, round: usize) -> InjectionAction {
    if observed < round {
        InjectionAction::Wait
    } else if observed == round {
        InjectionAction::Inject
    } else {
        InjectionAction::Skip
    }
}

pub(super) fn observe_interactive_marker(
    line: &str,
    next: &mut usize,
    order: InteractiveMarkerOrder,
) -> Result<bool> {
    let Some(index) = INTERACTIVE_MARKERS
        .iter()
        .position(|record| is_required_boot_marker(line, *record))
    else {
        return Ok(false);
    };
    let ordered = match order {
        InteractiveMarkerOrder::Strict => index == *next,
        InteractiveMarkerOrder::Mutation => index >= *next,
    };
    ensure!(
        ordered,
        "self-test interactive marker {} arrived while waiting for {}",
        index + 1,
        *next + 1
    );
    *next = index + 1;
    Ok(true)
}

pub(super) fn parse_selftest_verdict(line: &str) -> Option<Result<SelftestVerdict>> {
    let (severity, message) = log_message(line, LogSeverity::Info)
        .map(|message| (LogSeverity::Info, message))
        .or_else(|| {
            log_message(line, LogSeverity::Error).map(|message| (LogSeverity::Error, message))
        })?;
    let mut fields = message.strip_prefix(SELFTEST_VERDICT_PREFIX)?.split(' ');
    let version = core::iter::Iterator::next(&mut fields);
    let status = core::iter::Iterator::next(&mut fields);
    let passed = core::iter::Iterator::next(&mut fields);
    let failed = core::iter::Iterator::next(&mut fields);
    let expected = core::iter::Iterator::next(&mut fields);

    let parsed = (|| {
        if version != Some(SELFTEST_VERDICT_VERSION) {
            bail!("unsupported self-test verdict version");
        }
        let status = match status {
            Some("PASS") => VerdictStatus::Pass,
            Some("FAIL") => VerdictStatus::Fail,
            _ => bail!("invalid self-test verdict status"),
        };
        ensure!(
            matches!(
                (severity, status),
                (LogSeverity::Info, VerdictStatus::Pass)
                    | (LogSeverity::Error, VerdictStatus::Fail)
            ),
            "self-test verdict status has the wrong log severity"
        );
        if core::iter::Iterator::next(&mut fields).is_some() {
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
    let value = field
        .and_then(|field| field.strip_prefix(name))
        .context("missing self-test verdict count")?;
    usize::try_from(canonical_decimal(value).context("invalid self-test verdict count")?)
        .context("self-test verdict count does not fit usize")
}

fn parse_baseline_value(field: Option<&str>, name: &str) -> Result<u64> {
    let value = field
        .and_then(|field| field.strip_prefix(name))
        .with_context(|| format!("missing baseline field {name}"))?;
    canonical_decimal(value).with_context(|| format!("invalid baseline field {name}"))
}

pub(super) fn parse_performance_sample(line: &str) -> Option<Result<PerformanceSample>> {
    let message = log_message(line, LogSeverity::Info)?;
    let mut fields = message.strip_prefix(BASELINE_PREFIX)?.split(' ');
    let parsed = (|| {
        if core::iter::Iterator::next(&mut fields) != Some("v1") {
            bail!("unsupported foundation baseline version");
        }
        let sample = PerformanceSample {
            boot_ns: parse_baseline_value(core::iter::Iterator::next(&mut fields), "boot_ns=")?,
            frames_free: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "frames_free=",
            )?,
            heap_slab: parse_baseline_value(core::iter::Iterator::next(&mut fields), "heap_slab=")?,
            heap_large: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "heap_large=",
            )?,
            heap_live: parse_baseline_value(core::iter::Iterator::next(&mut fields), "heap_live=")?,
            handles_committed: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "handles_committed=",
            )?,
            handles_live: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "handles_live=",
            )?,
            trap_ticks: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "trap_ticks=",
            )?,
            storage_bytes: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "storage_bytes=",
            )?,
            storage_ticks: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "storage_ticks=",
            )?,
            counter_hz: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "counter_hz=",
            )?,
        };
        if core::iter::Iterator::next(&mut fields).is_some() {
            bail!("trailing fields in foundation baseline");
        }
        Ok(sample)
    })();
    Some(parsed)
}

pub(super) fn parse_stack_sample(line: &str) -> Option<Result<StackSample>> {
    let message = log_message(line, LogSeverity::Info)?;
    let mut fields = message.strip_prefix(NVME_STACK_PREFIX)?.split(' ');
    let parsed = (|| {
        if core::iter::Iterator::next(&mut fields) != Some("v2") {
            bail!("unsupported NVMe stack record version");
        }
        let sample = StackSample {
            used: parse_baseline_value(core::iter::Iterator::next(&mut fields), "used=")?,
            capacity: parse_baseline_value(core::iter::Iterator::next(&mut fields), "capacity=")?,
            headroom: parse_baseline_value(core::iter::Iterator::next(&mut fields), "headroom=")?,
            leaf: parse_baseline_value(core::iter::Iterator::next(&mut fields), "leaf=")?,
        };
        if core::iter::Iterator::next(&mut fields).is_some() {
            bail!("trailing fields in NVMe stack record");
        }
        Ok(sample)
    })();
    Some(parsed)
}

pub(super) fn observe_selftest_line(
    observation: &mut SerialObservation,
    line: &str,
) -> Result<PanicTransition> {
    if observation.verdict.is_some() {
        bail!("the self-test kernel printed serial data after its final verdict");
    }
    if let Some(transition) = observe_panic_line(
        &mut observation.panic,
        &mut observation.panic_complete,
        line,
    )? {
        return Ok(transition);
    }
    if let Some(name) = parse_failed_check(line) {
        if observation.failed.iter().any(|failed| failed == name) {
            bail!("the self-test kernel named one failed check more than once");
        }
        observation.failed.push(name.to_string());
        return Ok(PanicTransition::Unchanged);
    }
    if is_exact_log_message(line, LogSeverity::Info, STRICT_DMA_HANDOFF_MARKER) {
        if observation.strict_dma_handoff {
            bail!("the self-test kernel printed the strict DMA handoff more than once");
        }
        observation.strict_dma_handoff = true;
        return Ok(PanicTransition::Unchanged);
    }
    if is_exact_log_message(line, LogSeverity::Info, SELFTEST_QEMU_TCG_ISOLATION_MARKER) {
        if observation.qemu_tcg_isolation {
            bail!("the self-test kernel printed the TCG isolation decision more than once");
        }
        observation.qemu_tcg_isolation = true;
        return Ok(PanicTransition::Unchanged);
    }
    if let Some(sample) = parse_performance_sample(line) {
        let sample = sample?;
        if observation.baseline.replace(sample).is_some() {
            bail!("the self-test kernel printed more than one foundation baseline");
        }
        return Ok(PanicTransition::Unchanged);
    }
    if let Some(sample) = parse_stack_sample(line) {
        let sample = sample?;
        if observation.nvme_stack.replace(sample).is_some() {
            bail!("the self-test kernel printed more than one NVMe stack record");
        }
        return Ok(PanicTransition::Unchanged);
    }
    let Some(verdict) = parse_selftest_verdict(line) else {
        return Ok(PanicTransition::Unchanged);
    };
    let verdict = verdict?;
    if observation.verdict.replace(verdict).is_some() {
        bail!("the self-test kernel printed more than one final verdict");
    }
    Ok(PanicTransition::Unchanged)
}

pub(super) fn correlate_selftest(
    exit_code: Option<i32>,
    observation: SerialObservation,
) -> Result<SelftestOutcome> {
    if let Some(transcript) = completed_panic(observation.panic, observation.panic_complete)? {
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
    if verdict.status == VerdictStatus::Pass && !observation.qemu_tcg_isolation {
        bail!("self-test passed without consuming the exact QEMU TCG isolation policy");
    }
    if verdict.status == VerdictStatus::Pass && !observation.strict_dma_handoff {
        bail!("self-test passed without the strict QEMU DMA-handoff policy");
    }
    if verdict.status == VerdictStatus::Pass {
        let sample = observation
            .nvme_stack
            .context("self-test passed without an NVMe stack record")?;
        ensure!(
            sample.capacity == NVME_STACK_CAPACITY,
            "NVMe stack record named a {}-byte capacity, expected {NVME_STACK_CAPACITY}",
            sample.capacity
        );
        ensure!(
            sample.used.checked_add(sample.headroom) == Some(sample.capacity),
            "NVMe stack record does not conserve its capacity"
        );
        ensure!(
            sample.leaf <= sample.used,
            "NVMe stack record's live-leaf use {} exceeds its painted high-water mark {}",
            sample.leaf,
            sample.used
        );
        ensure!(
            sample.headroom >= NVME_STACK_MIN_HEADROOM,
            "NVMe candidates retained only {} bytes of stack headroom",
            sample.headroom
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

// Registers are the only hang evidence when the serial log is empty.
pub(super) fn report_hang(qmp: Option<&mut Qmp>) {
    let Some(qmp) = qmp else {
        println!("[xtask] no QMP session: where the kernel hung is unknown");
        return;
    };
    let deadline = Instant::now() + MONITOR_TIMEOUT;
    let Ok(state) = qmp.human("info registers", deadline, &NOT_CANCELLED) else {
        println!("[xtask] no monitor reply: where the kernel hung is unknown");
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
    let Ok(code) = qmp.human(&format!("x/8i 0x{rip}"), deadline, &NOT_CANCELLED) else {
        return;
    };
    for line in code
        .lines()
        .filter(|line| line.trim_start().starts_with("0x"))
    {
        println!("[xtask] hung in:  {}", line.trim());
    }
}

/// QMP on the child's own standard streams, which only the harness holds
/// (D-346). Serial output therefore uses a separate private OS transport.
pub(super) const QMP_ON_STDIO: [&str; 2] = ["-qmp", "stdio"];

/// Negotiate QMP over the piped standard streams of a freshly spawned QEMU.
pub(super) fn attach_qmp(
    child: &mut Child,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<Qmp> {
    let input = child.stdin.take().context("taking QEMU's stdin pipe")?;
    let output = child.stdout.take().context("taking QEMU's stdout pipe")?;
    Qmp::negotiate(input, output, deadline, cancelled)
}

/// One run's private, backpressured serial transport.
///
/// Windows QEMU creates a duplex named pipe. Its default DACL and a fresh
/// 256-bit name keep the endpoint private to this run; the kernel's output
/// can fill only the operating system's bounded pipe buffer. Unix QEMU uses
/// a socket beneath the private run directory. Neither route writes guest
/// bytes to an unbounded host file (D-367).
pub(super) struct SerialChannel {
    #[cfg(windows)]
    pub(super) name: String,
    #[cfg(unix)]
    pub(super) socket: PathBuf,
    #[cfg(unix)]
    reservation: PathBuf,
    closed: Arc<AtomicBool>,
}

impl SerialChannel {
    pub(super) fn create(target: &Path) -> Result<Self> {
        #[cfg(windows)]
        {
            let _ = target;
            let mut random = [0u8; 32];
            getrandom::fill(&mut random)
                .map_err(|error| anyhow!("obtaining a private QEMU serial-pipe name: {error}"))?;
            let mut suffix = String::with_capacity(random.len() * 2);
            for byte in random {
                use core::fmt::Write as _;
                write!(suffix, "{byte:02x}").expect("writing to a String cannot fail");
            }
            Ok(Self {
                name: format!("wizard-bedrock-serial-{}-{suffix}", std::process::id()),
                closed: Arc::new(AtomicBool::new(false)),
            })
        }

        #[cfg(unix)]
        {
            let (_, reservation, guard) = reserve_run_file(target, "serial", "guard")?;
            drop(guard);
            let socket = reservation.with_extension("sock");
            ensure!(
                !socket.exists(),
                "QEMU serial socket name is already occupied"
            );
            Ok(Self {
                socket,
                reservation,
                closed: Arc::new(AtomicBool::new(false)),
            })
        }

        #[cfg(not(any(windows, unix)))]
        {
            let _ = target;
            bail!("automated QEMU serial transport is unsupported on this host")
        }
    }

    /// The bounded character device that carries the guest's first serial port.
    pub(super) fn arguments(&self) -> [OsString; 4] {
        #[cfg(windows)]
        let backend = OsString::from(format!("pipe,id=serial0,path={}", self.name));
        #[cfg(unix)]
        let backend = OsString::from(format!(
            "socket,id=serial0,path={},server=on,wait=on",
            self.socket.to_string_lossy().replace(',', ",,")
        ));
        [
            OsString::from("-chardev"),
            backend,
            OsString::from("-serial"),
            OsString::from("chardev:serial0"),
        ]
    }

    /// Connect after QEMU creates its endpoint, refusing an exited or silent
    /// emulator rather than opening some ambient transport.
    pub(super) fn connect(&self, child: &mut Child, deadline: Instant) -> Result<SerialReader> {
        loop {
            #[cfg(windows)]
            let opened = OpenOptions::new()
                .read(true)
                .open(format!(r"\\.\pipe\{}", self.name))
                .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>);
            #[cfg(unix)]
            let opened = std::os::unix::net::UnixStream::connect(&self.socket)
                .map(|socket| Box::new(socket) as Box<dyn Read + Send>);
            #[cfg(not(any(windows, unix)))]
            let opened: std::io::Result<Box<dyn Read + Send>> =
                Err(std::io::Error::other("unsupported serial transport"));

            match opened {
                Ok(inner) => {
                    return Ok(SerialReader {
                        inner,
                        closed: Arc::clone(&self.closed),
                    });
                }
                Err(error) => {
                    if let Some(status) = child.try_wait().context("waiting for QEMU serial")? {
                        return Err(error).with_context(|| {
                            format!("QEMU exited before its serial transport opened ({status})")
                        });
                    }
                    if Instant::now() >= deadline {
                        return Err(error).context("QEMU did not open its serial transport");
                    }
                    std::thread::sleep(SERIAL_POLL);
                }
            }
        }
    }

    /// Record that QEMU has exited, so an observed pipe close is clean EOF.
    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

impl Drop for SerialChannel {
    fn drop(&mut self) {
        self.close();
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.socket);
            let _ = std::fs::remove_file(&self.reservation);
        }
    }
}

/// The host side of the bounded serial transport.
pub(super) struct SerialReader {
    pub(super) inner: Box<dyn Read + Send>,
    pub(super) closed: Arc<AtomicBool>,
}

impl Read for SerialReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.inner.read(buffer) {
            Ok(0) => await_serial_close(&self.closed),
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                await_serial_close(&self.closed)
            }
            outcome => outcome,
        }
    }
}

fn await_serial_close(closed: &AtomicBool) -> std::io::Result<usize> {
    let deadline = Instant::now() + SERIAL_CLOSE_GRACE;
    loop {
        if closed.load(Ordering::Acquire) {
            return Ok(0);
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "QEMU closed its serial transport before it exited",
            ));
        }
        std::thread::sleep(SERIAL_POLL);
    }
}

/// How often a serial connector looks for QEMU's freshly created endpoint.
const SERIAL_POLL: Duration = Duration::from_millis(10);
/// Enough for the parent loop to observe a QEMU exit before pipe EOF is judged
/// premature; the parent polls every 50 ms.
const SERIAL_CLOSE_GRACE: Duration = Duration::from_millis(250);

/// One guest record and one complete run are separately bounded. Once the
/// reader refuses the aggregate, OS backpressure bounds any bytes QEMU can
/// still write before the parent terminates it.
const MAX_SERIAL_LINE_BYTES: usize = 64 * 1024;
const MAX_SERIAL_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn read_serial_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    total: &mut usize,
) -> Result<usize> {
    read_serial_line_with_limits(reader, line, total, MAX_SERIAL_LINE_BYTES, MAX_SERIAL_BYTES)
}

pub(super) fn read_serial_line_with_limits(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    total: &mut usize,
    line_limit: usize,
    total_limit: usize,
) -> Result<usize> {
    line.clear();
    loop {
        let available = reader.fill_buf().context("buffering QEMU serial output")?;
        if available.is_empty() {
            return Ok(line.len());
        }
        let take = available
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(available.len(), |newline| newline + 1);
        let line_bytes = line
            .len()
            .checked_add(take)
            .context("QEMU serial line length overflowed")?;
        ensure!(
            line_bytes <= line_limit,
            "QEMU serial line exceeded the {line_limit}-byte limit"
        );
        let aggregate = total
            .checked_add(take)
            .context("QEMU serial transcript length overflowed")?;
        ensure!(
            aggregate <= total_limit,
            "QEMU serial transcript exceeded the {total_limit}-byte limit"
        );
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        *total = aggregate;
        if line.last() == Some(&b'\n') {
            return Ok(line.len());
        }
    }
}

/// Render untrusted guest bytes without passing terminal controls through to
/// the operator's console. The parser still receives the original bytes.
pub(super) fn escaped_serial_line(line: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let visible = line.strip_suffix(b"\n").unwrap_or(line);
    let mut escaped = String::with_capacity(visible.len());
    for &byte in visible {
        if (b' '..=b'~').contains(&byte) {
            escaped.push(char::from(byte));
        } else {
            escaped.push('\\');
            escaped.push('x');
            escaped.push(char::from(HEX[usize::from(byte >> 4)]));
            escaped.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    escaped
}

pub(super) fn echo_serial_line(line: &[u8]) {
    println!("{}", escaped_serial_line(line));
    let _ = Write::flush(&mut std::io::stdout());
}
