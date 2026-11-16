use core::sync::atomic::{AtomicU8, Ordering};

use stack_layout::StackLayout;
use wost::arch::current::{apic, cpu, percpu, xstate};
use wost::boot::{HandoffError, Processors};
use wost::sync::{Leaf, SpinLock};

use crate::thread::ThreadContext;
use crate::{acpi, info, interrupts, mem, thread, time, warn};

// The boot processor and up to fifteen more; a larger machine starts its first fifteen.
pub(crate) const MAX_PROCESSORS: usize = 16;

const STACK_BYTES: u64 = 32 * 1024;

const GUARD_BYTES: u64 = 4096;

const STEP_ROUNDS: u32 = 2_000;

// Two-processor TCG on a loaded host is slow; a processor that has not answered by then is not online.
const ANSWER_NS: u64 = 5_000_000_000;

const STARTED: u8 = 0;
const READY: u8 = 1;
const PASSED: u8 = 2;
const REFUSED: u8 = 3;

static STAGES: [AtomicU8; MAX_PROCESSORS] = [const { AtomicU8::new(STARTED) }; MAX_PROCESSORS];

#[derive(Clone, Copy, Debug)]
struct Posture {
    protections: cpu::Protections,
    speculation: cpu::Speculation,
}

static EXPECTED: SpinLock<Leaf, Option<Posture>> = SpinLock::new(None);

// Runs before the first user root, which freezes the online set; started processors only park.
pub(crate) fn bring_up(
    context: &mut ThreadContext,
    processors: Result<Processors, HandoffError>,
    table: Option<acpi::ProcessorTable>,
) {
    start_others(context, processors, table);
    let online = percpu::online_count();
    info!("cpu: {online} processors online");
}

fn start_others(
    context: &mut ThreadContext,
    processors: Result<Processors, HandoffError>,
    table: Option<acpi::ProcessorTable>,
) {
    let processors = match processors {
        Ok(processors) => processors,
        Err(reason) => {
            info!("cpu: no other processor started: {reason}");
            return;
        }
    };
    let x2apic = processors.x2apic();
    let mut waiting = processors.waiting().peekable();
    if waiting.peek().is_none() {
        return;
    }
    if !x2apic {
        warn!("cpu: the bootloader left x2apic off; no other processor starts");
        return;
    }
    let Some(table) = table else {
        warn!("cpu: no madt processor table; no other processor starts");
        return;
    };
    let Some(speculation) = interrupts::boot_speculation() else {
        warn!("cpu: no boot speculation posture; no other processor starts");
        return;
    };
    if !time::is_calibrated() {
        warn!(
            "cpu: the clock is not calibrated, so no counter can be checked; no other processor starts"
        );
        return;
    }
    let Some(stacks) = thread::boot_stack_layout().and_then(|boot| {
        StackLayout::new(boot.end(), STACK_BYTES, GUARD_BYTES, MAX_PROCESSORS - 1)
    }) else {
        warn!("cpu: no stack window for other processors; none starts");
        return;
    };
    *EXPECTED.lock_leaf() = Some(Posture {
        protections: cpu::Protections::enabled(),
        speculation,
    });
    // Sibling threads share a core's buffers, so without immunity only thread zero of a core runs.
    let thread_mask = (1u32 << cpu::smt_bits()) - 1;
    let one_thread_per_core = speculation.needs_buffer_clear();

    let mut started = 0;
    for waiting in waiting {
        let apic_id = waiting.apic_id();
        let listed = (0..table.count())
            .filter_map(|index| table.processor(index))
            .any(|processor| processor.apic_id() == apic_id);
        if !listed {
            warn!(
                "cpu: processor {apic_id:#x} is not in the madt; it stays parked by the bootloader"
            );
            continue;
        }
        if one_thread_per_core && apic_id & thread_mask != 0 {
            info!(
                "cpu: processor {apic_id:#x} is a sibling thread on a processor that leaks across them; it stays parked by the bootloader"
            );
            continue;
        }
        let Some(slot) = stacks.slot(started) else {
            warn!(
                "cpu: more than {MAX_PROCESSORS} processors; {apic_id:#x} and later stay parked by the bootloader"
            );
            return;
        };
        started += 1;
        let joining = match mem::start_processor(context, waiting, &slot, application) {
            Ok(joining) => joining,
            Err(reason) => {
                warn!("cpu: processor {apic_id:#x} not started: {reason}");
                continue;
            }
        };
        let index = joining.index();
        let stage = &STAGES[index];
        if !answers(stage, READY) {
            warn!("cpu: processor {apic_id:#x} never reached its counter check; it is not online");
            continue;
        }
        let forward = time::counter_steps_forward(STEP_ROUNDS);
        if !answers(stage, PASSED) || !forward {
            warn!(
                "cpu: processor {apic_id:#x} does not match the boot processor or its counter; it is not online"
            );
            continue;
        }
        if percpu::admit(joining) {
            info!("cpu: processor {index} (apic {apic_id:#x}) is online and parked");
        } else {
            warn!("cpu: processor {apic_id:#x} came after the first user root; it is not online");
        }
    }
}

// True once `stage` reads `wanted`; false at a refusal or after `ANSWER_NS`.
fn answers(stage: &AtomicU8, wanted: u8) -> bool {
    let deadline = time::now_ns().saturating_add(ANSWER_NS);
    loop {
        match stage.load(Ordering::Acquire) {
            seen if seen == wanted => return true,
            REFUSED => return false,
            _ if time::now_ns() >= deadline => return false,
            _ => core::hint::spin_loop(),
        }
    }
}

// The started processor, on its own stack and entry area: check, step the counter, park.
fn application(index: usize) -> ! {
    let matches = matches_boot_processor() && time::counter_matches_owner();
    STAGES[index].store(READY, Ordering::Release);
    let forward = time::counter_steps_forward(STEP_ROUNDS);
    let verdict = if matches && forward { PASSED } else { REFUSED };
    STAGES[index].store(verdict, Ordering::Release);
    cpu::halt_forever()
}

fn matches_boot_processor() -> bool {
    let Some(expected) = *EXPECTED.lock_leaf() else {
        return false;
    };
    cpu::establish_control_state().is_ok()
        && cpu::enable_protections() == expected.protections
        && xstate::join()
        && cpu::enable_speculation_controls() == expected.speculation
        && apic::join().is_ok()
}
