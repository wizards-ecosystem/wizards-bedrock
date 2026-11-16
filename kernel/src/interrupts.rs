use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use boot_record::{Fact, Record};
use wost::arch::current::{apic, cpu, interrupt, pic};

use crate::arch::current::vector;
use crate::routing::ExternalControllersMasked;
use crate::{info, iommu, time, timepage, warn};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UserIsolation {
    Closed,
    Shared,
    Paired,
}

impl UserIsolation {
    const fn bits(self) -> u8 {
        match self {
            Self::Closed => 0,
            Self::Shared => 1,
            Self::Paired => 2,
        }
    }

    const fn from_bits(bits: u8) -> Self {
        match bits {
            1 => Self::Shared,
            2 => Self::Paired,
            _ => Self::Closed,
        }
    }
}

static USER_ISOLATION: AtomicU8 = AtomicU8::new(UserIsolation::Closed.bits());

pub(crate) fn user_isolation() -> UserIsolation {
    UserIsolation::from_bits(USER_ISOLATION.load(Ordering::Acquire))
}

static MASKABLE_INTERRUPTS_READY: AtomicBool = AtomicBool::new(false);

pub(crate) fn maskable_interrupts_are_ready() -> bool {
    MASKABLE_INTERRUPTS_READY.load(Ordering::Acquire)
}

// Calibration runs before `sti`, so no window is preempted; only its failure vetoes nothing.
pub(crate) fn start(external_controllers: Option<&ExternalControllersMasked>) {
    if let Err(reason) = pic::silence() {
        warn!("legacy controllers not silenced: {reason}; interrupts stay off");
        return;
    }
    info!("pic: legacy controllers remapped and masked");

    apply_speculation_controls();

    match apic::init(vector::LOCAL_APIC) {
        Ok(local) => info!("apic: {local}"),
        Err(reason) => {
            warn!("local apic: {reason}; interrupts stay off");
            if matches!(reason, apic::InitError::NoExtendedMode) {
                info!("{}", Record::Fact(Fact::X2apicAbsent));
            }
            return;
        }
    }

    start_clock();
    publish_time_page();

    if external_controllers.is_none() {
        warn!("ioapic: complete mask proof missing; interrupts stay off");
        return;
    }
    if !PCI_INTERRUPT_BOUNDARY.load(Ordering::Acquire) || !iommu::is_interrupt_boundary_active() {
        warn!("iommu: complete PCI interrupt boundary missing; interrupts stay off");
        return;
    }

    if let Err(reason) = apic::enable_after_pending_audit() {
        warn!("apic: pending interrupt audit refused sti: {reason}; interrupts stay off");
        return;
    }
    MASKABLE_INTERRUPTS_READY.store(true, Ordering::Release);
    info!("cpu: accepting interrupts");
}

fn apply_speculation_controls() {
    let speculation = cpu::enable_speculation_controls();
    info!("cpu: {speculation}");
    if speculation.has_predictor_barrier() {
        info!("cpu: branch predictions cleared on every change of user address space");
    } else {
        info!(
            "cpu: no predictor barrier (ibpb); one user address space can train another's branch predictions"
        );
    }
    match cpu::shared_kernel_mapping_reason(&speculation) {
        Some(reason) => {
            let isolation = if cfg!(feature = "selftest") {
                // Self-test runs the paired path even where QEMU's TCG tuple admits shared roots.
                UserIsolation::Paired
            } else {
                UserIsolation::Shared
            };
            USER_ISOLATION.store(isolation.bits(), Ordering::Release);
            if isolation == UserIsolation::Paired {
                info!(
                    "cpu: user isolation paired roots: forced for self-test; shared proof retained: {reason}"
                );
                info!("{}", Record::Fact(Fact::IsolationPairedForced));
            } else {
                info!("cpu: user isolation shared kernel mappings: {reason}");
                info!("{}", Record::Fact(Fact::IsolationShared));
            }
        }
        None => {
            USER_ISOLATION.store(UserIsolation::Paired.bits(), Ordering::Release);
            info!("cpu: user isolation paired roots: selected; shared mappings refused");
        }
    }

    // An unreadable `MD_CLEAR` errs toward executing `verw`.
    if !speculation.needs_buffer_clear() {
        info!(
            "cpu: no buffer clear on return to ring 3; the processor reports immunity to everything it clears"
        );
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
}

fn start_clock() {
    match time::init() {
        Ok(report) => {
            let apic_source_hz = report.apic_source_hz();
            info!("time: apic source at {apic_source_hz} hz; {report}");
            // Loud: this is the one clock property the kernel cannot check.
            if !report.counter_is_invariant() {
                warn!(
                    "time: the counter is not advertised invariant; the clock is only as steady as this machine's power management"
                );
            }
        }
        Err(reason) => warn!("clock: {reason}; uptime stays at zero, nothing preempts"),
    }
}

// A program reading `TIME` must find the mode word, not a page fault.
fn publish_time_page() {
    match timepage::root().start() {
        Ok(()) => info!(
            "time: world clock page v{} at {:#x}, world time from 0 in steps of {} ns",
            native_abi::timepage::VERSION,
            native_abi::address::TIME,
            timepage::PRECISION_NS
        ),
        Err(reason) => warn!("time: {reason}; ring 3 stays closed"),
    }
}

pub(crate) static PCI_INTERRUPT_BOUNDARY: AtomicBool = AtomicBool::new(false);
