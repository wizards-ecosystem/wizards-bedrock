use super::*;

/// Long enough for a reply, short enough not to extend a failing run.
pub(super) const MONITOR_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long QEMU has to greet and accept capabilities after it starts.
pub(super) const QMP_NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(10);

/// A kernel that neither passes nor fails within `timeout` is a hang, which is
/// itself a failure — so the emulator is killed rather than left running.
///
/// QMP uses the owned child's standard streams; serial uses a private named
/// pipe on Windows or a private Unix socket. The injector presses keys after
/// their serial markers. The owner cancels, stops QEMU and bounds worker joins
/// on every result path, including when a control write stops making progress.
pub fn run_selftest(paths: &Paths, image: &Path, timeout: Duration) -> Result<()> {
    let (outcome, _) =
        observe_selftest_run(paths, image, timeout, true, InteractiveMarkerOrder::Strict)?;
    require_selftest(outcome)
}

/// Boot `image` headless and preserve a valid red or panic outcome for a
/// mutation caller. A missing earlier interaction may be skipped so its later
/// exact verdict remains observable; ordinary self-test boots stay strict.
pub fn observe_selftest(paths: &Paths, image: &Path, timeout: Duration) -> Result<SelftestOutcome> {
    observe_selftest_run(
        paths,
        image,
        timeout,
        true,
        InteractiveMarkerOrder::Mutation,
    )
    .map(|(outcome, _)| outcome)
}

/// Boot a self-test image and require its one versioned baseline record.
pub(crate) fn measure_performance(
    paths: &Paths,
    image: &Path,
    timeout: Duration,
) -> Result<PerformanceSample> {
    let (outcome, baseline) =
        observe_selftest_run(paths, image, timeout, false, InteractiveMarkerOrder::Strict)?;
    require_selftest(outcome)?;
    baseline.context("a clean self-test boot emitted no foundation baseline")
}

fn observe_selftest_run(
    paths: &Paths,
    image: &Path,
    timeout: Duration,
    echo_serial: bool,
    marker_order: InteractiveMarkerOrder,
) -> Result<(SelftestOutcome, Option<PerformanceSample>)> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
    let coverage = RunDmaCoverage::create(paths.target_dir(), Remapping::Present)?;
    let serial = SerialChannel::create(paths.target_dir())?;

    println!("[xtask] booting the self-test kernel");

    let child = Command::new(&installation.emulator)
        .current_dir(vars.directory())
        .args(machine_args(Remapping::Present))
        .args(coverage.arguments())
        .args(["-display", "none", "-no-reboot"])
        .args(serial.arguments())
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .args(QMP_ON_STDIO)
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    let mut machine = QemuLifetime::new(child, stop_qemu);
    let injection_cancelled = Arc::clone(&machine.cancelled);
    let mut forwarder = None;
    let mut injector = None;
    let observed = (|| -> Result<SelftestTermination> {
        let child = &mut machine.child;
        let reader = serial.connect(child, Instant::now() + QMP_NEGOTIATION_TIMEOUT)?;
        let markers_seen = Arc::new(AtomicUsize::new(0));
        let panicked = Arc::new(AtomicBool::new(false));
        let serial_failed = Arc::new(AtomicBool::new(false));
        forwarder = Some(forward_serial(
            reader,
            Arc::clone(&markers_seen),
            Arc::clone(&panicked),
            Arc::clone(&serial_failed),
            echo_serial,
            marker_order,
        ));
        let deadline = Instant::now() + timeout;
        injector = Some(inject_on_marker(
            child,
            markers_seen,
            deadline,
            Arc::clone(&injection_cancelled),
        )?);
        loop {
            if let Some(status) = child.try_wait().context("waiting for QEMU")? {
                return Ok(SelftestTermination::Exited(status.code()));
            }
            if panicked.load(Ordering::Acquire) {
                return Ok(SelftestTermination::Panicked);
            }
            if serial_failed.load(Ordering::Acquire) {
                return Ok(SelftestTermination::SerialFailed);
            }
            if Instant::now() >= deadline {
                return Ok(SelftestTermination::TimedOut);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    })();

    machine.cancel();
    let mut injection_result = Ok(());
    let mut reporter = None;
    let finished_injector = if matches!(observed, Ok(SelftestTermination::TimedOut))
        && injector.as_ref().is_some_and(JoinHandle::is_finished)
    {
        injector.take()
    } else {
        None
    };
    if let Some(finished) = finished_injector {
        match finish_injector(finished, &injection_cancelled, Instant::now()) {
            Ok(mut qmp) => {
                let worker = std::thread::spawn(move || report_hang(qmp.as_mut()));
                worker_finished_by(&worker, Instant::now() + MONITOR_TIMEOUT);
                reporter = Some(worker);
            }
            Err(error) => injection_result = Err(error),
        }
    }

    let stopped = machine.stop();
    serial.close();
    let join_deadline = Instant::now() + QEMU_WORKER_STOP_TIMEOUT;
    if let Some(worker) = injector {
        injection_result = finish_injector(worker, &injection_cancelled, join_deadline).map(drop);
    }
    let report_result = reporter.map_or(Ok(()), |worker| {
        join_qemu_worker(worker, join_deadline, "hang reporter")
    });
    let serial_result = forwarder
        .map(|worker| finish_selftest_serial(worker, join_deadline))
        .transpose();
    let cleanup = combine_qemu_cleanup(stopped, injection_result);
    let cleanup = combine_qemu_cleanup(cleanup, report_result);
    let outcome = observed.and_then(|termination| selftest_exit(termination, timeout));
    let outcome =
        combine_qemu_observation(outcome, serial_result).and_then(|(code, observation)| {
            let observation = observation.context("self-test has no serial observation")?;
            let baseline = observation.baseline;
            correlate_selftest(code, observation).map(|outcome| (outcome, baseline))
        });
    combine_qemu_cleanup(outcome, cleanup)
}

pub(super) enum SelftestTermination {
    Exited(Option<i32>),
    Panicked,
    SerialFailed,
    TimedOut,
}

pub(super) fn selftest_exit(
    termination: SelftestTermination,
    timeout: Duration,
) -> Result<Option<i32>> {
    match termination {
        SelftestTermination::Exited(code) => Ok(code),
        SelftestTermination::Panicked => Ok(None),
        SelftestTermination::SerialFailed => bail!("the self-test serial reader failed"),
        SelftestTermination::TimedOut => bail!(
            "self-test did not finish within {} seconds",
            timeout.as_secs()
        ),
    }
}

pub(super) struct QemuLifetime<C> {
    pub(super) child: C,
    stop: fn(&mut C) -> Result<()>,
    pub(super) cancelled: Arc<AtomicBool>,
    stop_attempted: bool,
}

impl<C> QemuLifetime<C> {
    pub(super) fn new(child: C, stop: fn(&mut C) -> Result<()>) -> Self {
        Self {
            child,
            stop,
            cancelled: Arc::new(AtomicBool::new(false)),
            stop_attempted: false,
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(super) fn stop(&mut self) -> Result<()> {
        self.cancel();
        self.stop_attempted = true;
        (self.stop)(&mut self.child)
    }
}

impl<C> Drop for QemuLifetime<C> {
    fn drop(&mut self) {
        if !self.stop_attempted {
            let _ = self.stop();
        }
    }
}

pub(super) fn combine_qemu_cleanup<T>(outcome: Result<T>, cleanup: Result<()>) -> Result<T> {
    combine_qemu_observation(outcome, cleanup).map(|(value, ())| value)
}

pub(super) fn combine_qemu_observation<T, U>(
    outcome: Result<T>,
    cleanup: Result<U>,
) -> Result<(T, U)> {
    match (outcome, cleanup) {
        (Ok(value), Ok(evidence)) => Ok((value, evidence)),
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(anyhow!("{error:#}; QEMU cleanup also failed: {cleanup:#}"))
        }
    }
}

pub(super) const QEMU_WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(5);

fn worker_finished_by<T>(worker: &JoinHandle<T>, deadline: Instant) -> bool {
    while !worker.is_finished() {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
    true
}

pub(super) fn join_qemu_worker<T>(
    worker: JoinHandle<T>,
    deadline: Instant,
    name: &str,
) -> Result<T> {
    ensure!(
        worker_finished_by(&worker, deadline),
        "QEMU {name} did not stop before its join deadline"
    );
    worker
        .join()
        .map_err(|_| anyhow!("the QEMU {name} panicked"))
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
    serial: impl Read + Send + 'static,
    markers_seen: Arc<AtomicUsize>,
    panicked: Arc<AtomicBool>,
    serial_failed: Arc<AtomicBool>,
    echo: bool,
    marker_order: InteractiveMarkerOrder,
) -> JoinHandle<Result<SerialObservation>> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(serial);
        let mut line = Vec::new();
        let mut total = 0;
        let mut observation = SerialObservation::default();
        let mut next_interactive_marker = 0;
        loop {
            line.clear();
            let read = match read_serial_line(&mut reader, &mut line, &mut total) {
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
                echo_serial_line(&line);
            }
            match observe_selftest_line(&mut observation, &text) {
                Ok(transition) => {
                    transition.publish(&panicked);
                    match observe_interactive_marker(
                        &text,
                        &mut next_interactive_marker,
                        marker_order,
                    ) {
                        Ok(true) => markers_seen.store(next_interactive_marker, Ordering::Release),
                        Ok(false) => {}
                        Err(error) => {
                            serial_failed.store(true, Ordering::Release);
                            return Err(error).context("reading interactive serial evidence");
                        }
                    }
                }
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading the kernel's self-test verdict");
                }
            }
        }
    })
}

fn finish_selftest_serial(
    forwarder: JoinHandle<Result<SerialObservation>>,
    deadline: Instant,
) -> Result<SerialObservation> {
    join_qemu_worker(forwarder, deadline, "self-test serial reader")?
}

/// Inject each key after its marker without letting a QMP stall delay the boot
/// deadline. The owner stops QEMU before bounded joins so a blocked control
/// write cannot hold cleanup; a finished thread can supply hang diagnostics.
fn inject_on_marker(
    child: &mut Child,
    markers_seen: Arc<AtomicUsize>,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) -> Result<JoinHandle<Option<Qmp>>> {
    let input = child.stdin.take().context("taking QEMU's stdin pipe")?;
    let output = child.stdout.take().context("taking QEMU's stdout pipe")?;
    Ok(std::thread::spawn(move || {
        let mut session = None;
        let outcome = (|| -> Result<usize> {
            let negotiation = deadline.min(Instant::now() + QMP_NEGOTIATION_TIMEOUT);
            let qmp = session.insert(Qmp::negotiate(input, output, negotiation, &cancelled)?);
            let mut injected = 0;
            for (round, keys) in [
                (1, STORAGE_DELAY_QCODES),
                (2, INJECTED_QCODES),
                (3, TRANSLATED_QCODES),
            ] {
                let action = loop {
                    let observed = markers_seen.load(Ordering::Acquire);
                    let action = injection_action(observed, round);
                    if action != InjectionAction::Wait {
                        break action;
                    }
                    if cancelled.load(Ordering::Acquire) {
                        bail!("QMP injection was cancelled");
                    }
                    if Instant::now() >= deadline {
                        bail!("QMP injection reached the run deadline");
                    }
                    std::thread::sleep(POLL_INTERVAL);
                };
                if action == InjectionAction::Skip {
                    continue;
                }
                keys.iter()
                    .try_for_each(|qcode| qmp.inject_key(qcode, deadline, &cancelled))?;
                injected += keys.len();
            }
            Ok(injected)
        })();
        match outcome {
            Ok(injected) => println!("[xtask] qmp: {injected} keystrokes injected"),
            Err(reason) => {
                println!("[xtask] qmp: {reason:#}; the keystroke check will report it");
            }
        }
        session
    }))
}

/// Cancel and join the self-test injector, recovering its QMP session.
fn finish_injector(
    injector: JoinHandle<Option<Qmp>>,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<Option<Qmp>> {
    cancelled.store(true, Ordering::Release);
    join_qemu_worker(injector, deadline, "QMP injector")
}

/// How often to check whether QEMU has exited. Short enough that a passing run
/// is not padded noticeably, long enough not to spin.
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(50);
const QEMU_STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn stop_qemu(child: &mut Child) -> Result<()> {
    stop_qemu_with(child, child_exited, terminate_child, QEMU_STOP_TIMEOUT)
}

fn child_exited(child: &mut Child) -> std::io::Result<bool> {
    Child::try_wait(child).map(|status| status.is_some())
}

fn terminate_child(child: &mut Child) -> std::io::Result<()> {
    Child::kill(child)
}

pub(super) fn stop_qemu_with<C>(
    child: &mut C,
    exited: fn(&mut C) -> std::io::Result<bool>,
    terminate: fn(&mut C) -> std::io::Result<()>,
    timeout: Duration,
) -> Result<()> {
    let initial = exited(child).context("checking whether QEMU exited");
    if matches!(initial, Ok(true)) {
        return Ok(());
    }
    let stopped = terminate_qemu_with(child, exited, terminate, timeout);
    combine_qemu_cleanup(initial.map(drop), stopped)
}

fn terminate_qemu_with<C>(
    child: &mut C,
    exited: fn(&mut C) -> std::io::Result<bool>,
    terminate: fn(&mut C) -> std::io::Result<()>,
    timeout: Duration,
) -> Result<()> {
    let termination = terminate(child).context("terminating QEMU");
    if termination.is_err() {
        match exited(child).context("checking QEMU after termination failed") {
            Ok(true) => return Ok(()),
            Ok(false) => return termination,
            Err(error) => return combine_qemu_cleanup(termination, Err(error)),
        }
    }

    let deadline = Instant::now()
        .checked_add(timeout)
        .context("QEMU stop deadline is not representable")?;
    loop {
        if exited(child).context("polling terminated QEMU")? {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            bail!(
                "QEMU did not exit within {} ms after termination",
                timeout.as_millis()
            );
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
}

pub(super) fn irq_count(report: &str, irq: u8) -> Option<u64> {
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
        if <u8 as core::str::FromStr>::from_str(label.trim()).ok() != Some(irq) {
            continue;
        }
        let mut fields = count.split_whitespace();
        let count =
            <u64 as core::str::FromStr>::from_str(core::iter::Iterator::next(&mut fields)?).ok()?;
        total = total.checked_add(count)?;
    }
    recognized.then_some(total)
}

pub(super) fn keyboard_irq_count(qmp: &mut Qmp) -> Result<u64> {
    let report = qmp
        .human(
            "info irq",
            Instant::now() + SHIPPED_CHALLENGE_TIMEOUT,
            &NOT_CANCELLED,
        )
        .context("querying keyboard IRQ progress")?;
    irq_count(&report, 1).context("QEMU monitor returned no IRQ statistics")
}
