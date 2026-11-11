use super::*;

/// The self-test build exits at the debug-exit device, so only this boot covers what the
/// shipped kernel does afterwards: it must settle, call from ring 3, print every required
/// platform record and never panic.
pub fn run_boot_check(paths: &Paths, image: &Path, settle: Duration) -> Result<()> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
    let coverage = RunDmaCoverage::create(paths.target_dir(), Remapping::Present)?;
    let serial = SerialChannel::create(paths.target_dir())?;

    println!("[xtask] booting the shipped kernel");

    let mut child = Command::new(&installation.emulator)
        .current_dir(vars.directory())
        .args(machine_args(Remapping::Present))
        .args(coverage.arguments())
        .args(["-display", "none", "-no-reboot"])
        .args(serial.arguments())
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

    let serial_reader = match serial.connect(&mut child, Instant::now() + QMP_NEGOTIATION_TIMEOUT) {
        Ok(reader) => reader,
        Err(error) => {
            stop_qemu(&mut child).context("stopping QEMU after serial connection failed")?;
            return Err(error);
        }
    };

    let qmp = attach_qmp(
        &mut child,
        Instant::now() + QMP_NEGOTIATION_TIMEOUT,
        &NOT_CANCELLED,
    );
    let settled = Arc::new(AtomicBool::new(false));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let required: Vec<_> = SHIPPED_REQUIRED_MARKERS
        .iter()
        .copied()
        .map(|marker| (marker, Arc::new(AtomicBool::new(false))))
        .collect();
    let forwarder = watch_serial(
        serial_reader,
        Arc::clone(&settled),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        required.clone(),
    );

    let outcome = qmp.and_then(|mut qmp| {
        watch(
            &mut child,
            &panicked,
            &serial_failed,
            BOOT_TIMEOUT,
            Some(&settled),
        )?;
        require_masked_keyboard(&mut child, &mut qmp, &panicked, &serial_failed)?;
        watch(&mut child, &panicked, &serial_failed, settle, None)
    });

    let stop_outcome = stop_qemu(&mut child);
    serial.close();
    let serial_outcome = finish_boot_serial(forwarder, Instant::now() + QEMU_WORKER_STOP_TIMEOUT)
        .and_then(|serial_observation| {
            if let Some(transcript) =
                completed_panic(serial_observation.panic, serial_observation.panic_complete)?
            {
                bail!("the shipped kernel panicked during the settle window: {transcript}");
            }
            Ok(())
        });

    let outcome = combine_qemu_cleanup(outcome, serial_outcome);
    combine_qemu_cleanup(outcome, stop_outcome)?;
    require_records_seen(&required, "shipped")?;
    println!("[xtask] shipped kernel falls back, boots, responds, and settles");
    Ok(())
}

fn require_masked_keyboard(
    child: &mut Child,
    qmp: &mut Qmp,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
) -> Result<()> {
    let before = keyboard_irq_count(qmp)?;
    qmp.inject_key(
        SHIPPED_CHALLENGE_QCODE,
        Instant::now() + SHIPPED_CHALLENGE_TIMEOUT,
        &NOT_CANCELLED,
    )
    .context("injecting the shipped-kernel liveness key")?;
    watch(
        child,
        panicked,
        serial_failed,
        SHIPPED_CHALLENGE_WINDOW,
        None,
    )?;
    let after = keyboard_irq_count(qmp)?;
    ensure!(
        after == before,
        "the shipped kernel unmasked legacy keyboard IRQ1 ({before} -> {after})"
    );
    println!("[xtask] shipped liveness: keyboard input left IRQ1 masked at {after}");
    Ok(())
}

fn require_records_seen(
    required: &[(RequiredLogRecord, Arc<AtomicBool>)],
    kernel: &str,
) -> Result<()> {
    for (record, seen) in required {
        if !seen.load(Ordering::Acquire) {
            bail!(
                "the {kernel} kernel settled without reporting {:?}",
                record.message
            );
        }
    }
    Ok(())
}

/// Three human keystroke rounds fit inside it, so an unanswered round ends as the kernel's own
/// red check and the verdict stays the kernel's.
const BOOT_TIMEOUT: Duration = Duration::from_secs(180);

/// No x2APIC and no optional SMEP, SMAP or UMIP; the last `-cpu` wins over `machine_args`'s.
pub(super) const DEGRADED_CPU: [&str; 2] = [
    "-cpu",
    "qemu64,-x2apic,-smep,-smap,-umip,hypervisor=on,tcg-cpuid=on",
];

#[derive(Clone, Copy, Debug)]
struct DegradedProfile {
    announcement: &'static str,
    cpu: Option<&'static [&'static str; 2]>,
    remapping: Remapping,
    required: &'static [RequiredLogRecord],
}

const NO_PCI_PROFILE: DegradedProfile = DegradedProfile {
    announcement: "booting a kernel with no PCI configuration or x2apic",
    cpu: Some(&DEGRADED_CPU),
    remapping: Remapping::Present,
    required: &[],
};

const NO_X2APIC_PROFILE: DegradedProfile = DegradedProfile {
    announcement: "booting the shipped kernel with no x2apic",
    cpu: Some(&DEGRADED_CPU),
    remapping: Remapping::Present,
    required: DEGRADED_REQUIRED_MARKERS,
};

const UNISOLATED_PROFILE: DegradedProfile = DegradedProfile {
    announcement: "booting the shipped kernel with no DMA-remapping unit",
    cpu: None,
    remapping: Remapping::Absent,
    required: UNISOLATED_REQUIRED_MARKERS,
};

/// The other boots keep PCI configuration, and a precondition only ever met is
/// indistinguishable from one never checked; this one keeps its exact terminal panic.
pub fn observe_degraded(paths: &Paths, image: &Path, settle: Duration) -> Result<DegradedOutcome> {
    observe_profile(paths, image, settle, NO_PCI_PROFILE)
}

/// PCI configuration stays live here, so the DMA handoff can still be fenced.
pub fn observe_no_x2apic(paths: &Paths, image: &Path, settle: Duration) -> Result<DegradedOutcome> {
    observe_profile(paths, image, settle, NO_X2APIC_PROFILE)
}

/// With no remapping unit, storage must refuse every candidate instead of running unisolated.
pub fn observe_unisolated(
    paths: &Paths,
    image: &Path,
    settle: Duration,
) -> Result<DegradedOutcome> {
    observe_profile(paths, image, settle, UNISOLATED_PROFILE)
}

fn observe_profile(
    paths: &Paths,
    image: &Path,
    settle: Duration,
    profile: DegradedProfile,
) -> Result<DegradedOutcome> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
    let coverage = RunDmaCoverage::create(paths.target_dir(), profile.remapping)?;

    println!("[xtask] {}", profile.announcement);

    let mut child = Command::new(&installation.emulator)
        .current_dir(vars.directory())
        .args(machine_args(profile.remapping))
        .args(coverage.arguments())
        .args(profile.cpu.map_or(&[][..], |cpu| &cpu[..]))
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
    let required: Vec<_> = profile
        .required
        .iter()
        .copied()
        .map(|marker| (marker, Arc::new(AtomicBool::new(false))))
        .collect();
    let forwarder = watch_serial(
        stdout,
        Arc::clone(&settled),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        required.clone(),
    );

    let outcome = watch_degraded(&mut child, &panicked, &serial_failed, &settled, settle);

    let stop_outcome = stop_qemu(&mut child);
    let serial_outcome = finish_boot_serial(forwarder, Instant::now() + QEMU_WORKER_STOP_TIMEOUT)
        .and_then(|serial_observation| {
            let panic =
                completed_panic(serial_observation.panic, serial_observation.panic_complete)?;
            Ok((panic, serial_observation.no_x2apic_before_panic))
        });

    let (_, (panic, no_x2apic_diagnosed)) = combine_qemu_cleanup(
        combine_qemu_observation(outcome, serial_outcome),
        stop_outcome,
    )?;
    if let Some(transcript) = panic {
        return Ok(DegradedOutcome::Panicked {
            transcript,
            no_x2apic_diagnosed,
        });
    }
    require_records_seen(&required, "degraded")?;
    Ok(DegradedOutcome::Settled)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DegradedWatch {
    Complete,
    Panicked,
}

fn watch_degraded(
    child: &mut Child,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
    settled: &AtomicBool,
    settle: Duration,
) -> Result<DegradedWatch> {
    match watch_degraded_window(child, panicked, serial_failed, BOOT_TIMEOUT, Some(settled))? {
        DegradedWatch::Panicked => Ok(DegradedWatch::Panicked),
        DegradedWatch::Complete => {
            watch_degraded_window(child, panicked, serial_failed, settle, None)
        }
    }
}

fn watch_degraded_window(
    child: &mut Child,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
    window: Duration,
    until: Option<&AtomicBool>,
) -> Result<DegradedWatch> {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        if panicked.load(Ordering::Acquire) {
            return Ok(DegradedWatch::Panicked);
        }
        if let Some(status) = child.try_wait().context("waiting for QEMU")? {
            bail!("QEMU exited before the degraded kernel settled: {status}");
        }
        if serial_failed.load(Ordering::Acquire) {
            bail!("reading the degraded kernel's serial output failed");
        }
        if until.is_some_and(|marker| marker.load(Ordering::Acquire)) {
            return Ok(DegradedWatch::Complete);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    match until {
        Some(_) => bail!("the degraded kernel never reached its steady state ({SETTLED_MARKER:?})"),
        None => Ok(DegradedWatch::Complete),
    }
}

/// With `until`, the window is a deadline that flag must beat; without, the whole window is
/// watched for a panic or an early exit.
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
            bail!("the shipped kernel panicked: see the log above");
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

fn watch_serial(
    serial: impl Read + Send + 'static,
    settled: Arc<AtomicBool>,
    panicked: Arc<AtomicBool>,
    serial_failed: Arc<AtomicBool>,
    required: Vec<(RequiredLogRecord, Arc<AtomicBool>)>,
) -> JoinHandle<Result<BootSerialObservation>> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(serial);
        let mut line = Vec::new();
        let mut total = 0;
        let mut observation = BootSerialObservation::default();
        loop {
            line.clear();
            let read = match read_serial_line(&mut reader, &mut line, &mut total) {
                Ok(read) => read,
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading shipped QEMU serial output");
                }
            };
            if read == 0 {
                return Ok(observation);
            }
            let text = String::from_utf8_lossy(&line);
            echo_serial_line(&line);
            if is_settled_record(&text) {
                settled.store(true, Ordering::Release);
            }
            for (record, seen) in &required {
                let present = is_required_boot_marker(&text, *record);
                if present {
                    seen.store(true, Ordering::Release);
                }
            }
            match observe_boot_line(&mut observation, &text) {
                Ok(transition) => transition.publish(&panicked),
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading the kernel's boot transcript");
                }
            }
        }
    })
}

pub(super) fn finish_boot_serial(
    forwarder: JoinHandle<Result<BootSerialObservation>>,
    deadline: Instant,
) -> Result<BootSerialObservation> {
    join_qemu_worker(forwarder, deadline, "boot serial reader")?
}
