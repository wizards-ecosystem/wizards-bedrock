//! The Wizard's Bedrock kernel — entry point.
//!
//! Booted by Limine (base revision 4, D-42) on x86_64, higher-half. Every privileged
//! operation goes through `wost`; this crate contains no hand-written `unsafe`.
//! `deny` rather than `forbid` because [`wost::entry!`] expands its own
//! `#[allow(unsafe_code)]` glue here — see D-21.

#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![cfg_attr(
    not(feature = "selftest"),
    deny(clippy::expect_used, clippy::unwrap_used)
)]

/// The heap `wost::mm::heap` provides. Nothing in the kernel may allocate
/// before `mem::init` has run, which is why the boot order in `kernel_main`
/// puts it before everything that might.
extern crate alloc;

mod acpi;
mod arch;
mod banner;
mod console;
mod dev;
mod devfs;
mod fs;
mod handle;
mod input;
mod iommu;
mod log;
mod mem;
mod nvme;
mod object;
mod panic;
mod pci;
mod queue;
#[cfg(feature = "selftest")]
mod selftest;
mod theme;
mod thread;
mod time;
mod timepage;
mod tmpfs;
mod user;
mod vfs;
mod wait;

use core::fmt::Write;
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::uart::UartError;
use wost::arch::current::{apic, cpu, interrupt, pci as platform_pci, pic, segment, task, xstate};
use wost::boot::BootInfo;
use wost::sync::SpinLock;

use crate::arch::current::{ioapic, vector};

::wost::entry!(kernel_main);

fn kernel_main(boot: BootInfo) -> ! {
    let serial = dev::init_uart();
    info!("The Wizard's Bedrock awakens.");

    match boot.framebuffer {
        Ok(framebuffer) => dev::init_console(framebuffer),
        Err(reason) => warn!("framebuffer: {reason}"),
    }

    // Checked only once both consoles are up, so that a machine with neither a
    // serial cable nor a supported bootloader still shows why it stopped.
    assert!(
        boot.base_revision_supported,
        "bootloader rejected our Limine base revision"
    );
    info!("limine: base revision supported");

    // The fault path exists before anything that can fault, so a wrong
    // assumption prints a report instead of resetting (`006`).
    segment::init();
    arch::current::trap::init();
    info!("cpu: descriptor tables installed");
    let protections = cpu::enable_protections();
    info!("cpu: protections {protections}");

    // Before any thread but this one exists, because every switch saves and
    // restores from here on (D-108).
    match xstate::init() {
        Some(extended) => info!(
            "cpu: extended state {} bytes, xcr0 {:#x}{}",
            extended.bytes,
            extended.mask,
            if extended.compacted {
                ", compacted"
            } else {
                ""
            }
        ),
        None => warn!("cpu: extended state unavailable; ring 3 stays closed"),
    }

    // Before ring 3 exists: `AC` survives interrupt delivery and is writable
    // at CPL 3, so a trap could otherwise arrive with SMAP off (D-101).
    if protections.smap {
        interrupt::arm_access_clear();
    }

    // After the fault path, because this is the first write through an
    // address the bootloader chose; before everything else, which may want a frame.
    mem::init(boot.memory_map, boot.direct_map);
    let interrupts = mem::init_paging(boot.memory_map, boot.direct_map, boot.image, boot.rsdp);
    if let Ok(rsdp) = boot.rsdp {
        iommu::discover(rsdp);
    }

    // The bootloader's stack has nothing below it and sits inside the
    // unsplittable direct map, so the only guard is to leave it (D-92).
    *HANDOFF.lock_leaf() = Some(Handoff {
        serial,
        interrupts,
        module: boot.module,
    });
    match mem::map_boot_stack() {
        Some(stack) => task::continue_on_stack(stack, boot_continue, 1),
        None => {
            warn!(
                "boot: staying on the bootloader's stack, which has no guard; ring 3 stays closed"
            );
            boot_continue(0)
        }
    }
}

/// What the boot thread carries across the change of stack.
///
/// A static rather than an argument because the old stack is abandoned: a
/// pointer into it would be a pointer into memory nothing owns any more.
struct Handoff {
    serial: Result<(), UartError>,
    interrupts: Option<acpi::Interrupts>,
    /// The initrd, which the boot thread reads before it changes stacks and
    /// the loader reads long after.
    module: Result<&'static [u8], wost::boot::HandoffError>,
}

static HANDOFF: SpinLock<Option<Handoff>> = SpinLock::new(None);

/// Whether the boot thread reached its guarded stack before user launch.
static BOOT_STACK_GUARDED: AtomicBool = AtomicBool::new(false);

/// Whether process construction may leave the boot stack.
pub(crate) fn user_launch_is_safe() -> bool {
    BOOT_STACK_GUARDED.load(Ordering::Acquire)
}

static SHARED_KERNEL_MAPPINGS_SAFE: AtomicBool = AtomicBool::new(false);

/// Whether process construction may install the kernel half in user tables.
pub(crate) fn shared_kernel_mappings_are_safe() -> bool {
    SHARED_KERNEL_MAPPINGS_SAFE.load(Ordering::Acquire)
}

static MASKABLE_INTERRUPTS_READY: AtomicBool = AtomicBool::new(false);

pub(crate) fn maskable_interrupts_are_ready() -> bool {
    MASKABLE_INTERRUPTS_READY.load(Ordering::Acquire)
}

/// Run one check in the state left by a refused boot-stack allocation.
#[cfg(feature = "selftest")]
pub(crate) fn with_unguarded_boot_stack_for_test<T>(check: impl FnOnce() -> T) -> T {
    let guarded = BOOT_STACK_GUARDED.swap(false, Ordering::AcqRel);
    let result = check();
    BOOT_STACK_GUARDED.store(guarded, Ordering::Release);
    result
}

/// Run one check without an accepted shared-mapping proof.
#[cfg(feature = "selftest")]
pub(crate) fn without_shared_kernel_mapping_proof_for_test<T>(check: impl FnOnce() -> T) -> T {
    let safe = SHARED_KERNEL_MAPPINGS_SAFE.swap(false, Ordering::AcqRel);
    let result = check();
    SHARED_KERNEL_MAPPINGS_SAFE.store(safe, Ordering::Release);
    result
}

/// Run one check with the boot interrupt admission revoked.
#[cfg(feature = "selftest")]
pub(crate) fn without_maskable_interrupt_admission_for_test<T>(check: impl FnOnce() -> T) -> T {
    let ready = MASKABLE_INTERRUPTS_READY.swap(false, Ordering::AcqRel);
    let result = check();
    MASKABLE_INTERRUPTS_READY.store(ready, Ordering::Release);
    result
}

/// The rest of the boot, on a guarded stack.
///
/// # Panics
///
/// If the handoff is missing, which would mean this ran twice.
extern "C" fn boot_continue(guarded: usize) -> ! {
    BOOT_STACK_GUARDED.store(guarded == 1, Ordering::Release);
    let Some(Handoff {
        serial,
        interrupts,
        module,
    }) = HANDOFF.lock_leaf().take()
    else {
        panic!("the boot handoff is gone; boot_continue ran twice");
    };

    // The heap exists from here; the console's shadow is its first consumer,
    // so scrolling stops reading uncached device memory (D-48).
    dev::attach_back_buffer();
    let routing = prepare_keyboard_routing(interrupts).into_masked();

    #[cfg(feature = "refuse-pci")]
    refuse_pci_for_degraded_boot();
    survey_the_bus();
    survey_the_filesystems();
    start_interrupts(routing.as_ref());
    input::init();
    route_keyboard(routing);
    input::start_poller();
    queue::start_worker();

    dev::with_console(|console| {
        for line in banner::LINES {
            console.write_line(line);
        }
        // The one message that cannot go to the log it is about.
        if let Err(reason) = serial {
            console.write_line("");
            let _ = writeln!(console, "  no serial log: {reason}");
        }
    });

    user::keep(module);

    #[cfg(feature = "selftest")]
    selftest::run(serial);

    // After interrupts are on, because a halting thread needs them to wake;
    // idempotent, because the self-checks start it earlier.
    thread::start_idle();

    // The shipped path must exercise user entry too (D-116). A refusal is
    // diagnostic rather than fatal, so the kernel remains interactive.
    match user::run() {
        Ok(program) => info!("user: program running in slot {}", program.slot),
        Err(reason) => warn!("user: no program: {reason}"),
    }
    info!(
        "kernel: reading the keyboard, uptime {} ms",
        time::uptime_ms()
    );
    echo_keystrokes()
}

/// The boot thread's last job: block on the keyboard, and draw what arrives.
///
/// Nothing polls and nothing spins: this thread is not runnable until a
/// keystroke wakes it (D-61), the idle thread halts meanwhile, and no timer
/// is armed — so a kernel at a prompt takes one interrupt per keystroke.
///
/// Drawing is here rather than in the handler because a glyph can cost a
/// scroll (D-63).
fn echo_keystrokes() -> ! {
    let mut context = thread::ThreadContext::require();
    loop {
        let keystroke = input::read_key(&mut context);
        debug!("key: {:?} {:?}", keystroke.event.key, keystroke.event.state);
        if let Some(character) = keystroke.character {
            dev::with_console(|console| {
                let _ = console.write_char(character);
            });
        }
    }
}

/// Bring up the interrupt controller, start the clock, and accept interrupts.
///
/// Each step is a precondition for the next, and every one of them can decline:
/// a kernel that cannot take interrupts is worth far more than one that takes
/// them into a controller it never configured. In order — stop the legacy
/// controllers delivering anything, give the local APIC its vectors and turn
/// it on, measure the clocks, and only then set the flag. Calibration sits
/// where it does for both of its neighbours: it needs the APIC's registers
/// answering, and running before the `sti` is what makes the measurement
/// unpreemptible. It arms nothing — the timer is the scheduler's to arm, and
/// nothing has asked it for anything yet. Alone among the steps, its failure
/// vetoes nothing: a kernel without a clock still wants its keyboard.
fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {
    if let Err(reason) = pic::silence() {
        warn!("legacy controllers not silenced: {reason}; interrupts stay off");
        return;
    }
    info!("pic: legacy controllers remapped and masked");

    // One MSR write at boot (D-81); the report matters as much, because this
    // is the only place a machine's speculation claims are visible.
    let speculation = cpu::enable_speculation_controls();
    info!("cpu: {speculation}");
    match cpu::shared_kernel_mapping_reason(&speculation) {
        Some(reason) => {
            SHARED_KERNEL_MAPPINGS_SAFE.store(true, Ordering::Release);
            info!("cpu: user isolation shared kernel mappings: {reason}");
        }
        None => {
            SHARED_KERNEL_MAPPINGS_SAFE.store(false, Ordering::Release);
            warn!("cpu: user isolation shared kernel mappings: refused; ring 3 stays closed");
        }
    }

    // Arm `verw` unless `MDS_NO` makes it pointless; an unreadable `MD_CLEAR`
    // errs toward executing it (D-98).
    if speculation.mds_no() {
        info!("cpu: no buffer clear on return to ring 3; the processor reports mds_no");
    } else {
        interrupt::enable_buffer_clear();
        let buffer_clear_reported = cpu::has_buffer_clear();
        info!(
            "cpu: buffers cleared on every return to ring 3{}",
            if buffer_clear_reported {
                ""
            } else {
                " (md_clear unreported, so the flush may buy nothing here)"
            }
        );
    }

    match apic::init(vector::LOCAL_APIC) {
        Ok(local) => info!("apic: {local}"),
        Err(reason) => {
            warn!("local apic: {reason}; interrupts stay off");
            return;
        }
    }

    match time::init() {
        Ok(report) => {
            let apic_source_hz = report.apic_source_hz();
            info!("time: apic source at {apic_source_hz} hz; {report}");
            // Loud: this is the one clock property the kernel cannot check,
            // and a machine without the promise may be throttling underneath.
            if !report.counter_is_invariant() {
                warn!(
                    "time: the counter is not advertised invariant; the clock is only as steady as this machine's power management"
                );
            }
        }
        Err(reason) => warn!("clock: {reason}; uptime stays at zero, nothing preempts"),
    }

    // The page exists whether or not the clock does: a program reading `TIME`
    // must find `MODE_NONE`, not a page fault (D-166).
    if timepage::init(time::counter_hz()) {
        let mode = if time::is_calibrated() {
            timepage::MODE_COUNTER
        } else {
            timepage::MODE_NONE
        };
        assert_eq!(
            timepage::publish(mode, time::origin_ticks(), time::conversion()),
            timepage::Publication::Published,
            "the initialized time page refused its first publication"
        );
        info!(
            "time: published at {:#x}, {}",
            timepage::TIME,
            if mode == timepage::MODE_COUNTER {
                "readable without a call"
            } else {
                "revoked; readers must ask ring Uptime"
            }
        );
    } else {
        warn!("time: no page to publish the clock in; every query is ring Uptime");
    }

    #[cfg(feature = "selftest")]
    EXTERNAL_MASK_REFUSAL_PROVED.store(
        !external_interrupts_are_admitted(None) && !interrupt::are_enabled(),
        Ordering::Release,
    );
    if !external_interrupts_are_admitted(external_controllers) {
        warn!("ioapic: complete mask proof missing; interrupts stay off");
        return;
    }

    interrupt::enable();
    MASKABLE_INTERRUPTS_READY.store(true, Ordering::Release);
    info!("cpu: accepting interrupts");
}

const fn external_interrupts_are_admitted(mask: Option<&mem::ExternalControllersMasked>) -> bool {
    mask.is_some()
}

#[cfg(feature = "selftest")]
static EXTERNAL_MASK_REFUSAL_PROVED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(crate) fn unproven_external_controllers_veto_interrupts() -> bool {
    EXTERNAL_MASK_REFUSAL_PROVED.load(Ordering::Acquire)
}

#[cfg(feature = "refuse-pci")]
fn refuse_pci_for_degraded_boot() {
    assert!(
        wost::arch::current::port::pci_configuration().is_some(),
        "the degraded boot could not reserve the PCI configuration ports"
    );
}

/// Claim configuration space and say what is on the bus.
///
/// Declines rather than fails: a machine whose configuration ports are already
/// spoken for has no PCI as far as this kernel is concerned, and everything
/// built so far works without it.
///
/// The walk **recurses through bridges** — QEMU's q35 puts a DMI-to-PCI bridge
/// at `00:1e.0` and VMware places functions behind `pciBridge0`–`pciBridge7`,
/// so a scan of bus zero can miss a controller entirely (`012` §2, D-168).
fn survey_the_bus() {
    if !wost::arch::current::pci::init() {
        warn!("pci: the configuration ports are already claimed; no bus");
        return;
    }

    match pci::snapshot_topology() {
        Ok(()) => info!("pci: complete segment-zero topology sealed"),
        Err(reason) => warn!("pci: DMA topology refused: {reason}"),
    }

    // These checks need an ordinary unclaimed function. Run them before a
    // complete VT-d requester inventory deliberately owns every function in
    // the unit, and cache only their verdicts for the later report.
    #[cfg(feature = "selftest")]
    selftest::prepare_pci_claim_checks();

    let mut selection = StorageSelection::Seeking;
    let mut storage = 0;
    let surveyed = pci::enumerate(|function| {
        if function.class.is_nvme() {
            storage += 1;
            if matches!(selection, StorageSelection::Seeking) {
                selection = try_controller(function.at, storage);
            }
        }
    });

    match surveyed {
        Some(survey) => info!(
            "pci: {} functions across {} bus(es), {} bridge(s), highest {:#04x}, {storage} nvme",
            survey.functions, survey.buses, survey.bridges, survey.highest_bus
        ),
        None => {
            warn!("pci: configuration space did not answer");
            return;
        }
    }

    match selection {
        StorageSelection::Selected => mount_the_volume(),
        StorageSelection::Seeking if storage == 0 => {
            info!("pci: no nvme controller; nothing to claim")
        }
        StorageSelection::Seeking => info!("nvme: no usable controller"),
        StorageSelection::Halted => {
            warn!("nvme: selection stopped because a candidate could not be retired")
        }
    }
}

/// Whether enumeration should try another storage function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageSelection {
    Seeking,
    Selected,
    Halted,
}

/// Where a storage candidate stands between the stages of [`try_controller`].
#[derive(Debug)]
enum CandidateStage {
    /// Leased, mapped and verified disabled, with bus mastering still off;
    /// the flag records whether the sibling isolation stage has completed.
    Prepared(pci::Device, nvme::DisabledController, bool),
    /// Bus mastering admitted behind the installed domain.
    Admitted(pci::Device, nvme::BusMasterAdmittedController),
    /// A stage failed and only the lease remains, for [`retire_candidate`].
    Retire(pci::Device),
    /// Selection is decided and no lease remains to return.
    Settled(StorageSelection),
}

impl CandidateStage {
    /// Move the owners out, leaving selection halted until a stage returns.
    fn take(&mut self) -> Self {
        core::mem::replace(self, Self::Settled(StorageSelection::Halted))
    }

    /// The owners of a prepared candidate; any other stage stays in place.
    fn take_prepared(&mut self) -> Option<(pci::Device, nvme::DisabledController)> {
        match self.take() {
            Self::Prepared(device, prepared, true) => Some((device, prepared)),
            other => {
                *self = other;
                None
            }
        }
    }

    /// The owners of an admitted candidate; any other stage stays in place.
    fn take_admitted(&mut self) -> Option<(pci::Device, nvme::BusMasterAdmittedController)> {
        match self.take() {
            Self::Admitted(device, admitted) => Some((device, admitted)),
            other => {
                *self = other;
                None
            }
        }
    }
}

/// Claim one storage candidate and either commit or retire it.
///
/// Selection advances only after the failed candidate has stopped DMA,
/// unmapped its registers, returned its queues, restored its command word and
/// released its function and I/O claims (D-199). Every stage returns its
/// owners here, so one stage frame at a time sits on the boot stack (D-359).
fn try_controller(at: pci_config::Address, ordinal: usize) -> StorageSelection {
    #[cfg(feature = "selftest")]
    let stack = nvme::CandidateStackMeasurement::begin();
    let mut context = thread::ThreadContext::require();
    #[cfg(feature = "selftest")]
    let baseline = CandidateBaseline::capture(&mut context);
    #[cfg(feature = "selftest")]
    let bme_activity = nvme::activity_for_test();
    #[cfg(feature = "selftest")]
    let forced_before = pci::bus_master_refusal_was_observed();
    #[cfg(feature = "selftest")]
    NVME_CANDIDATE_ATTEMPTS.fetch_add(1, Ordering::Relaxed);

    // Stages borrow the candidate, so the debug frame holds one copy of its
    // owners rather than one per stage call.
    let mut stage = claim_candidate(at, ordinal, &mut context);
    let device = loop {
        if matches!(&stage, CandidateStage::Prepared(_, _, false)) {
            isolate_candidate(&mut stage, &mut context);
            continue;
        }
        stage = match stage {
            CandidateStage::Prepared(..) => admit_candidate(&mut stage, &mut context),
            CandidateStage::Admitted(..) => start_candidate(&mut stage, &mut context, ordinal),
            CandidateStage::Retire(device) => break device,
            CandidateStage::Settled(selection) => {
                #[cfg(feature = "selftest")]
                nvme::finish_candidate_stack(stack.as_ref());
                return selection;
            }
        };
    };
    let selection = retire_candidate(
        device,
        &mut context,
        #[cfg(feature = "selftest")]
        baseline,
        ordinal,
    );
    #[cfg(feature = "selftest")]
    if !forced_before && pci::bus_master_refusal_was_observed() {
        NVME_BME_REFUSAL_PROVED.store(
            selection == StorageSelection::Seeking
                && nvme::activity_for_test() == bme_activity
                && nvme::refused_bus_master_followed_disable_for_test(),
            Ordering::Release,
        );
    }
    #[cfg(feature = "selftest")]
    nvme::finish_candidate_stack(stack.as_ref());
    selection
}

/// Lease, map, prepare and isolate a candidate before it may bus-master.
fn claim_candidate(
    at: pci_config::Address,
    ordinal: usize,
    context: &mut thread::ThreadContext,
) -> CandidateStage {
    let mut device = match pci::lease(at) {
        Ok(device) => device,
        Err(reason) => return CandidateStage::Settled(refused_lease(at, reason)),
    };
    let (vendor, model) = device.identity();
    info!(
        "nvme: claimed {:02x}:{:02x}.{} — {vendor:04x}:{model:04x}",
        at.bus(),
        at.device(),
        at.function()
    );

    let registers = match admit_controller_window(&mut device, context) {
        ControllerWindowAdmission::Admitted(registers) => registers,
        ControllerWindowAdmission::Refused => return CandidateStage::Retire(device),
        ControllerWindowAdmission::Quarantined => {
            pci::retain_failed_release(device);
            return CandidateStage::Settled(StorageSelection::Halted);
        }
    };
    let prepared = match nvme::prepare(context, registers) {
        nvme::PrepareOutcome::Prepared(prepared) => prepared,
        nvme::PrepareOutcome::Failed(failure) => {
            return failed_preparation(failure, device, ordinal);
        }
    };
    CandidateStage::Prepared(device, prepared, false)
}

/// Establish unit-wide isolation without moving the prepared controller into
/// the same debug frame as VT-d's bounded inventory and table construction.
fn isolate_candidate(stage: &mut CandidateStage, context: &mut thread::ThreadContext) {
    let outcome = match stage {
        CandidateStage::Prepared(device, _, false) => iommu::establish(context, device),
        _ => return,
    };
    match outcome {
        iommu::EstablishOutcome::Established => {}
        iommu::EstablishOutcome::Unavailable => warn!("storage: no DMA isolation"),
        iommu::EstablishOutcome::Quarantined => {
            quarantine_isolation(stage);
            return;
        }
    }
    let CandidateStage::Prepared(_, _, isolated) = stage else {
        panic!("the isolation stage lost its prepared candidate");
    };
    *isolated = true;
}

/// Move uncertain isolation owners into their permanent quarantine only after
/// the VT-d establishment frame has returned.
fn quarantine_isolation(stage: &mut CandidateStage) {
    let CandidateStage::Prepared(device, prepared, false) = stage.take() else {
        panic!("an isolation quarantine lost its prepared candidate");
    };
    warn!("storage: DMA isolation state is uncertain; selection halted");
    nvme::keep_iommu_quarantine(prepared, device);
    *stage = CandidateStage::Settled(StorageSelection::Halted);
}

/// Report a refused lease, and whether selection may try the next function.
fn refused_lease(at: pci_config::Address, reason: pci::ClaimError) -> StorageSelection {
    let bus = at.bus();
    let device_number = at.device();
    let function = at.function();
    match reason {
        pci::ClaimError::ProbeRestore(recovery) => match recovery.retry() {
            pci::ProbeRecoveryOutcome::Recovered => {
                warn!(
                    "nvme: {:02x}:{:02x}.{} probe failed but restored on retry",
                    bus, device_number, function
                );
                StorageSelection::Seeking
            }
            pci::ProbeRecoveryOutcome::Refused(failure) => {
                warn!(
                    "nvme: {:02x}:{:02x}.{} probe restoration remains uncertain: {:?}",
                    bus,
                    device_number,
                    function,
                    failure.reason()
                );
                pci::retain_failed_probe(failure.into_recovery());
                StorageSelection::Halted
            }
            pci::ProbeRecoveryOutcome::FunctionRetained => {
                warn!(
                    "nvme: {:02x}:{:02x}.{} probe restored but its function claim remains retained",
                    bus, device_number, function
                );
                StorageSelection::Halted
            }
        },
        pci::ClaimError::FunctionRetained => {
            warn!(
                "nvme: {:02x}:{:02x}.{} preparation failed with its function claim retained",
                bus, device_number, function
            );
            StorageSelection::Halted
        }
        reason => {
            warn!(
                "nvme: {:02x}:{:02x}.{} could not be claimed: {reason}",
                bus, device_number, function
            );
            StorageSelection::Seeking
        }
    }
}

/// Keep an uncertain preparation with its lease, or hand the lease back.
fn failed_preparation(
    failure: nvme::PrepareFailure,
    device: pci::Device,
    ordinal: usize,
) -> CandidateStage {
    let reason = failure.reason();
    let Some(quarantine) = failure.into_quarantine() else {
        return CandidateStage::Retire(device);
    };
    warn!(
        "nvme: candidate {ordinal} left uncertain controller state after {}",
        reason
    );
    nvme::keep_prepare_quarantine(quarantine, device);
    CandidateStage::Settled(StorageSelection::Halted)
}

/// Admit bus mastering for a prepared candidate behind its installed domain.
fn admit_candidate(
    stage: &mut CandidateStage,
    context: &mut thread::ThreadContext,
) -> CandidateStage {
    let Some((mut device, prepared)) = stage.take_prepared() else {
        return stage.take();
    };
    let admitted = match prepared.admit_bus_master(&mut device) {
        nvme::BusMasterAdmission::Admitted(admitted) => admitted,
        nvme::BusMasterAdmission::Refused(failure) => {
            warn!(
                "nvme: bus master refused after verified disable: {}",
                failure.reason()
            );
            match failure.into_disabled().retire(&mut device, context) {
                nvme::ControllerRetirement::Retired => {}
                nvme::ControllerRetirement::Quarantined(quarantine) => {
                    nvme::keep_controller_quarantine(quarantine, device);
                    return CandidateStage::Settled(StorageSelection::Halted);
                }
            }
            return CandidateStage::Retire(device);
        }
    };
    info!("nvme: bus master on after verified controller disable");
    CandidateStage::Admitted(device, admitted)
}

/// Start an admitted controller, read block 0 through it, and keep it.
fn start_candidate(
    stage: &mut CandidateStage,
    context: &mut thread::ThreadContext,
    ordinal: usize,
) -> CandidateStage {
    let Some((mut device, admitted)) = stage.take_admitted() else {
        return stage.take();
    };
    let mut controller = match nvme::start(context, &mut device, admitted) {
        Ok(controller) => controller,
        Err(failure) => {
            let reason = failure.reason();
            match failure.into_retirement() {
                nvme::ControllerRetirement::Retired => return CandidateStage::Retire(device),
                nvme::ControllerRetirement::Quarantined(quarantine) => {
                    warn!(
                        "nvme: candidate {ordinal} left uncertain controller state after {}",
                        reason
                    );
                    nvme::keep_controller_quarantine(quarantine, device);
                    return CandidateStage::Settled(StorageSelection::Halted);
                }
            }
        }
    };

    // One block, to prove the whole path: queue built, command submitted,
    // doorbell rung, phase tag recognised, completion matched by identifier.
    let Some(segment) = wost::mm::allocate_contiguous(1, 1) else {
        warn!("nvme: no memory for a read buffer");
        match controller.retire(&mut device, context) {
            nvme::ControllerRetirement::Retired => {}
            nvme::ControllerRetirement::Quarantined(quarantine) => {
                nvme::keep_controller_quarantine(quarantine, device);
                return CandidateStage::Settled(StorageSelection::Halted);
            }
        }
        return CandidateStage::Retire(device);
    };
    let mut buffer = wost::mm::DmaSegment::from(segment);
    match controller.read_blocks(0, 1, &mut buffer) {
        Ok(()) => info!("nvme: read block 0"),
        Err(reason) => {
            warn!("nvme: block 0 could not be read: {reason}");
            let retirement = controller.retire(&mut device, context);
            iommu::report_faults();
            match retirement {
                nvme::ControllerRetirement::Retired => {}
                nvme::ControllerRetirement::Quarantined(quarantine) => {
                    nvme::keep_controller_quarantine(quarantine, device);
                    drop(buffer);
                    return CandidateStage::Settled(StorageSelection::Halted);
                }
            }
            drop(buffer);
            return CandidateStage::Retire(device);
        }
    }
    if let Err(reason) = device.commit() {
        warn!("nvme: pci command commit refused: {reason}");
        pci::retain_failed_commit(device);
        nvme::keep(controller, buffer);
        return CandidateStage::Settled(StorageSelection::Halted);
    }
    nvme::keep(controller, buffer);
    #[cfg(feature = "selftest")]
    NVME_SELECTED_ORDINAL.store(ordinal, Ordering::Release);
    CandidateStage::Settled(StorageSelection::Selected)
}

/// Finish one failed lease, stopping selection unless restoration is exact.
fn retire_candidate(
    mut device: pci::Device,
    context: &mut thread::ThreadContext,
    #[cfg(feature = "selftest")] baseline: CandidateBaseline,
    ordinal: usize,
) -> StorageSelection {
    if let Err(reason) = device.disable_bus_master_for_retirement() {
        warn!("nvme: candidate {ordinal} bus-master fence was refused once: {reason}");
        if let Err(reason) = device.disable_bus_master_for_retirement() {
            warn!(
                "nvme: candidate {ordinal} could not prove bus mastering off after retry: {reason}; selection halted"
            );
            pci::retain_failed_release(device);
            return StorageSelection::Halted;
        }
    }
    if let Err(reason) = device.settle_bus_master_fence_for_retirement() {
        warn!("nvme: candidate {ordinal} safe command restoration was refused once: {reason}");
        if let Err(reason) = device.settle_bus_master_fence_for_retirement() {
            warn!(
                "nvme: candidate {ordinal} could not settle its bus-master fence after retry: {reason}; selection halted"
            );
            pci::retain_failed_release(device);
            return StorageSelection::Halted;
        }
    }
    // BME is verified off, excluded from the restored command word and still
    // protected by the selected function claim while translation goes away.
    if !iommu::release_candidate(context) {
        warn!("nvme: candidate {ordinal} could not return its dma domain; selection halted");
        pci::retain_failed_release(device);
        return StorageSelection::Halted;
    }
    if let Some((reason, device)) = device.release(context).into_retained() {
        warn!(
            "nvme: candidate {ordinal} pci lease retirement was refused once: {}",
            reason
        );
        if let Some((reason, device)) = device.release(context).into_retained() {
            warn!(
                "nvme: candidate {ordinal} could not return its pci lease after retry: {}; selection halted",
                reason
            );
            pci::retain_failed_release(device);
            return StorageSelection::Halted;
        }
    }
    #[cfg(feature = "selftest")]
    if !baseline.is_restored(context) {
        warn!("nvme: candidate {ordinal} did not restore its resource baseline");
        return StorageSelection::Halted;
    }
    #[cfg(feature = "selftest")]
    NVME_CLEAN_RETIREMENTS.fetch_add(1, Ordering::Relaxed);
    info!("nvme: candidate {ordinal} retired cleanly; trying next");
    StorageSelection::Seeking
}

/// Resources a failed selection candidate must return exactly.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy)]
struct CandidateBaseline {
    frames: usize,
    claims: usize,
    mappings: Option<usize>,
}

#[cfg(feature = "selftest")]
impl CandidateBaseline {
    fn capture(_context: &mut thread::ThreadContext) -> Self {
        Self {
            frames: wost::mm::free_frames(),
            claims: pci::claims(),
            mappings: mem::device_mappings(_context),
        }
    }

    fn is_restored(self, _context: &mut thread::ThreadContext) -> bool {
        wost::mm::free_frames() == self.frames
            && pci::claims() == self.claims
            && mem::device_mappings(_context) == self.mappings
    }
}

#[cfg(feature = "selftest")]
static NVME_CANDIDATE_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static NVME_CLEAN_RETIREMENTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static NVME_SELECTED_ORDINAL: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static NVME_BME_REFUSAL_PROVED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static NVME_BAR_ADMISSION_PROVED: AtomicBool = AtomicBool::new(false);

/// QEMU's namespace-less first controller retired before its usable sibling.
#[cfg(feature = "selftest")]
fn a_later_nvme_controller_survives_an_earlier_failure() -> bool {
    NVME_CANDIDATE_ATTEMPTS.load(Ordering::Acquire) == 2
        && NVME_CLEAN_RETIREMENTS.load(Ordering::Acquire) == 1
        && NVME_SELECTED_ORDINAL.load(Ordering::Acquire) == 2
}

#[cfg(feature = "selftest")]
fn a_live_nvme_bar_obeyed_maxphyaddr() -> bool {
    NVME_BAR_ADMISSION_PROVED.load(Ordering::Acquire)
}

#[derive(Debug)]
enum ControllerWindowAdmission {
    Admitted(platform_pci::MappedBar),
    Refused,
    Quarantined,
}

fn admit_controller_window(
    device: &mut pci::Device,
    context: &mut thread::ThreadContext,
) -> ControllerWindowAdmission {
    // BAR0 is the register file; a controller whose first window is not
    // memory is one this driver cannot reach.
    let Some(window) = device.window(0) else {
        warn!("nvme: no bar0; nothing to map");
        return ControllerWindowAdmission::Refused;
    };
    let pci_config::Bar::Memory { address, wide, .. } = window.bar else {
        warn!("nvme: bar0 is not a memory window");
        return ControllerWindowAdmission::Refused;
    };
    info!(
        "nvme: bar0 is {} bytes of {}-bit memory at {address:#x}",
        window.size,
        if wide { 64 } else { 32 }
    );
    let admitted = match device.admit_memory_bar(0) {
        Ok(admitted) => admitted,
        Err(reason) => {
            warn!("nvme: bar0 physical range refused: {reason}");
            return ControllerWindowAdmission::Refused;
        }
    };
    let (page_start, page_end, physical_bits) = admitted.physical_admission();
    let mapping = match mem::map_pci_bar(context, admitted) {
        Ok(mapping) => mapping,
        Err(failure) => {
            match failure.reason() {
                Some(reason) => warn!("nvme: bar0 mapping refused: {reason}"),
                None => warn!("nvme: no active address space for bar0"),
            }
            if let Err(failure) = platform_pci::release_admitted_bar(failure.into_admitted()) {
                warn!("nvme: bar0 authority return refused: {failure}");
                mem::retain_pci_bar_authority(failure);
                return ControllerWindowAdmission::Quarantined;
            }
            return ControllerWindowAdmission::Refused;
        }
    };
    let disabled = match mapping.claim_disabled() {
        Ok(disabled) => disabled,
        Err(mapping) => {
            warn!("nvme: the installed bar0 mapping is absent from the active root");
            mem::retain_pci_bar_mapping(mapping);
            return ControllerWindowAdmission::Quarantined;
        }
    };
    let mapped = match device.enable_memory_bar(disabled) {
        pci::BarDecodeOutcome::Enabled(mapped) => mapped,
        pci::BarDecodeOutcome::Refused(failure) => {
            let reason = failure.reason();
            let retirement =
                mem::unmap_pci_bar_exact(context, failure.into_bar().into_releasable());
            warn!("nvme: memory-space decode refused: {reason}; mapping {retirement:?}");
            return match retirement {
                mem::PciBarRetirement::Retired => ControllerWindowAdmission::Refused,
                mem::PciBarRetirement::Quarantined => ControllerWindowAdmission::Quarantined,
            };
        }
    };
    info!(
        "nvme: bar0 pages {page_start:#x}..{page_end:#x} admitted below MAXPHYADDR {physical_bits}"
    );
    #[cfg(feature = "selftest")]
    NVME_BAR_ADMISSION_PROVED.store(
        page_start < page_end && page_end <= 1_u64 << physical_bits,
        Ordering::Release,
    );
    info!("nvme: memory-space decode on; bus master remains off");
    ControllerWindowAdmission::Admitted(mapped)
}

#[cfg(feature = "selftest")]
fn refused_bus_master_retires_before_nvme_start() -> bool {
    pci::bus_master_refusal_was_observed() && NVME_BME_REFUSAL_PROVED.load(Ordering::Acquire)
}

/// Mount through the VFS so the shipped path exercises its abstraction.
/// An absent volume is a valid empty machine, not a boot failure (D-171).
fn mount_the_volume() {
    let shape = fs::mount().map(|result| result.map(|volume| volume.shape()));
    match shape {
        None => {
            info!("fat32: no controller, so no volume");
            return;
        }
        Some(Err(reason)) => {
            warn!("fat32: the volume would not mount: {reason:?}");
            return;
        }
        Some(Ok((clusters, cluster_bytes))) => {
            info!("fat32: mounted — {clusters} clusters of {cluster_bytes} bytes");
        }
    }

    let Some(root) = vfs::root() else {
        return;
    };

    // The controller's ThreadLock is non-reentrant across this callback, so
    // names are resolved only after enumeration releases it (D-200).
    let mut files = 0usize;
    let mut directories = 0usize;
    let mut first = [0u8; 16];
    let mut first_len = 0usize;
    if let Err(reason) = vfs::enumerate(root, |name, is_directory| {
        if is_directory {
            directories += 1;
        } else {
            files += 1;
            if first_len == 0 && name.len() <= first.len() {
                first[..name.len()].copy_from_slice(name);
                first_len = name.len();
            }
        }
        true
    }) {
        warn!("fat32: the root would not list: {reason:?}");
        return;
    }
    info!("fat32: {files} file(s) and {directories} directory(ies) in the root");

    if first_len == 0 {
        return;
    }
    let name = &first[..first_len];
    match vfs::resolve(root, name) {
        Ok(vfs::Node::File(file)) => {
            let mut head = [0u8; 32];
            match vfs::read(file, 0, &mut head) {
                Ok(read) => info!(
                    "fat32: {} — read {read} of its bytes",
                    core::str::from_utf8(name).unwrap_or("?")
                ),
                Err(reason) => warn!("fat32: that file would not read: {reason:?}"),
            }
        }
        Ok(vfs::Node::Directory(_)) => warn!("fat32: a listed file resolved to a directory"),
        // A FAT volume has no devices, and the compiler says so: a new node
        // kind is an error in every match (D-171).
        Ok(vfs::Node::Device(_)) => warn!("fat32: a listed file resolved to a device"),
        Err(reason) => warn!("fat32: a listed name would not resolve: {reason:?}"),
    }
}

/// Say what the other two backends hold.
///
/// `014` asks for more than one backend before the VFS's abstraction is worth
/// believing, and this is the shipped kernel exercising all three through the
/// same three operations. It is an unconditional sibling of bus discovery,
/// because neither backend needs a device (D-172, D-198).
fn survey_the_filesystems() {
    tmpfs::init();

    let mut files = 0usize;
    let mut directories = 0usize;
    let listed = vfs::enumerate(vfs::tmp_root(), |_, is_directory| {
        if is_directory {
            directories += 1;
        } else {
            files += 1;
        }
        true
    });
    match listed {
        Ok(()) => {
            info!("tmpfs: {files} file(s) and {directories} directory(ies) at the root");
        }
        Err(reason) => warn!("tmpfs: the root would not list: {reason:?}"),
    }

    for device in devfs::DEVICES {
        match vfs::resolve(vfs::dev_root(), device.name()) {
            Ok(vfs::Node::Device(found)) => info!(
                "devfs: {} is {}",
                core::str::from_utf8(found.name()).unwrap_or("?"),
                found.describes()
            ),
            Ok(_) => warn!("devfs: a device resolved to something with bytes in it"),
            Err(reason) => warn!("devfs: a device would not resolve: {reason:?}"),
        }
    }
}

/// Route the keyboard's line through the I/O APIC, so keystrokes interrupt.
///
/// The polled path is safe only after every described controller was proven
/// masked. An incomplete inventory also vetoes `sti`.
fn route_keyboard(routing: Option<mem::ExternalControllersMasked>) {
    let Some(routing) = routing else {
        warn!("keyboard: external controllers are unproven; interrupts and routing stay off");
        return;
    };
    let routing = match routing.into_routing() {
        Some(routing) if interrupt::are_enabled() => routing,
        Some(routing) => {
            if !rollback_keyboard_route(routing) {
                warn!("keyboard: masked routing mapping remains quarantined; staying polled");
                return;
            }
            info!("keyboard: interrupts stayed off; a poller thread drains it instead");
            return;
        }
        None => {
            info!("keyboard: no io apic owns its line; a poller thread drains it instead");
            return;
        }
    };

    #[cfg(feature = "selftest")]
    let routing = prove_keyboard_routing_rollback(routing);
    commit_keyboard_route(routing);
}

#[derive(Debug)]
enum KeyboardRouteError {
    NotOwned(u32),
    Controller(ioapic::RoutingError),
    Keyboard(crate::arch::current::i8042::InterruptEnableError),
}

impl core::fmt::Display for KeyboardRouteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotOwned(gsi) => write!(f, "gsi {gsi} is not this controller's"),
            Self::Controller(reason) => core::fmt::Display::fmt(reason, f),
            Self::Keyboard(reason) => core::fmt::Display::fmt(reason, f),
        }
    }
}

fn configure_keyboard_route(routing: &mut mem::Routing) -> Result<u32, KeyboardRouteError> {
    let gsi = routing.interrupts.gsi_for(KEYBOARD_IRQ);
    if !routing.io_apic.handles(gsi) {
        return Err(KeyboardRouteError::NotOwned(gsi));
    }

    let wiring = match routing.interrupts.flags_for(KEYBOARD_IRQ) {
        0 => ioapic::Wiring::isa_default(),
        flags => ioapic::Wiring::from_flags(flags),
    };
    routing
        .io_apic
        .route(gsi, vector::KEYBOARD, wiring)
        .map_err(KeyboardRouteError::Controller)?;

    input::enable_interrupts().map_err(KeyboardRouteError::Keyboard)?;
    Ok(gsi)
}

fn commit_keyboard_route(mut routing: mem::Routing) {
    let gsi = match configure_keyboard_route(&mut routing) {
        Ok(gsi) => gsi,
        Err(reason) => {
            if !rollback_keyboard_route(routing) {
                warn!(
                    "keyboard: routing refused after {reason}; rollback authority remains quarantined; staying polled"
                );
                return;
            }
            warn!("keyboard: routing refused: {reason}; staying polled");
            return;
        }
    };
    input::take_interrupts();
    dev::keep_io_apic(
        routing.io_apic.retain(),
        #[cfg(feature = "selftest")]
        gsi,
    );
    info!(
        "keyboard: irq {KEYBOARD_IRQ} on gsi {gsi} routed to vector {:#x}",
        vector::KEYBOARD.get()
    );
}

fn rollback_keyboard_route(mut routing: mem::Routing) -> bool {
    let gsi = routing.interrupts.gsi_for(KEYBOARD_IRQ);
    if routing.io_apic.handles(gsi)
        && let Err(reason) = routing.io_apic.mask_and_verify(gsi)
    {
        warn!("keyboard: could not verify rollback mask for gsi {gsi}: {reason}");
        mem::retain_routing(routing);
        return false;
    }
    let Some(mut context) = thread::ThreadContext::claim() else {
        warn!("keyboard: rollback could not claim thread context");
        mem::retain_routing(routing);
        return false;
    };
    #[cfg(feature = "selftest")]
    let mappings_before = mem::device_mappings(&mut context);
    if !mem::release_routing(&mut context, routing) {
        warn!("keyboard: rollback could not retire its io-apic mapping");
        return false;
    }
    #[cfg(feature = "selftest")]
    let found = mem::device_mappings(&mut context);
    #[cfg(feature = "selftest")]
    if found != mappings_before.and_then(|count| count.checked_sub(1)) {
        warn!(
            "keyboard: rollback did not retire exactly one device mapping: before={mappings_before:?}, after={found:?}"
        );
        return false;
    }
    true
}

#[cfg(feature = "selftest")]
static ROUTING_ROLLBACK_PROVED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
fn prove_keyboard_routing_rollback(mut routing: mem::Routing) -> mem::Routing {
    let interrupts = routing.interrupts;
    input::refuse_interrupt_readback();
    let refused = matches!(
        configure_keyboard_route(&mut routing),
        Err(KeyboardRouteError::Keyboard(
            crate::arch::current::i8042::InterruptEnableError::RefusedRestored
        ))
    );
    let rolled_back = rollback_keyboard_route(routing);
    let proved = refused && rolled_back && !input::is_interrupt_driven();
    ROUTING_ROLLBACK_PROVED.store(proved, Ordering::Release);
    assert!(
        proved,
        "keyboard routing rollback was not exact: refused={refused}, retired={rolled_back}, interrupt_driven={}, config={:?}",
        input::is_interrupt_driven(),
        input::controller_config()
    );
    let prepared = prepare_keyboard_routing(Some(interrupts)).into_masked();
    let Some(routing) = prepared.and_then(mem::ExternalControllersMasked::into_routing) else {
        panic!("keyboard routing could not be remapped after rollback");
    };
    routing
}

#[cfg(feature = "selftest")]
pub(crate) fn keyboard_routing_rollback_is_exact() -> bool {
    ROUTING_ROLLBACK_PROVED.load(Ordering::Acquire)
}

/// Map and select the controller which owns the keyboard's resolved GSI.
///
/// This runs before [`start_interrupts`], so probing masks every described
/// controller while the CPU still refuses external delivery.
fn prepare_keyboard_routing(interrupts: Option<acpi::Interrupts>) -> mem::RoutingPreparation {
    let Some(interrupts) = interrupts else {
        warn!("ioapic: no complete firmware controller inventory; interrupts stay off");
        return mem::RoutingPreparation::unproven();
    };
    let gsi = interrupts.gsi_for(KEYBOARD_IRQ);
    let Some(mut context) = thread::ThreadContext::claim() else {
        warn!("ioapic: no thread context for mapping; interrupts stay off");
        return mem::RoutingPreparation::unproven();
    };
    mem::prepare_routing(&mut context, interrupts, gsi)
}

/// The legacy line the keyboard controller has always used.
const KEYBOARD_IRQ: u8 = 1;
