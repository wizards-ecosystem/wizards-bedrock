use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use nvme::{configuration, status};
use stack_layout::{Slot as StackSlot, StackUsage};
use wost::arch::current::cpu;
use wost::arch::current::paging::{self, PaintedStack};
use x86_paging::PAGE_SIZE;

use super::{BringUpError, CONTROLLER};
use crate::thread;

#[cfg(feature = "selftest")]
pub(super) static BME_PRECEDING_CC: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(feature = "selftest")]
pub(super) static BME_PRECEDING_CSTS: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(feature = "selftest")]
pub(super) static REFUSED_BME_PRECEDING_CC: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(feature = "selftest")]
pub(super) static REFUSED_BME_PRECEDING_CSTS: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(feature = "selftest")]
pub(super) static START_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
pub(super) static SUBMISSION_DOORBELLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
const STACK_METER_IDLE: usize = 0;
#[cfg(feature = "selftest")]
const STACK_METER_PREPARING: usize = 1;
#[cfg(feature = "selftest")]
const STACK_METER_ACTIVE: usize = 2;
#[cfg(feature = "selftest")]
const STACK_METER_OBSERVING: usize = 3;
#[cfg(feature = "selftest")]
const STACK_METER_FINISHING: usize = 4;

#[cfg(feature = "selftest")]
static STACK_METER_STATE: AtomicUsize = AtomicUsize::new(STACK_METER_IDLE);
#[cfg(feature = "selftest")]
static STACK_LOWEST: AtomicU64 = AtomicU64::new(u64::MAX);
#[cfg(feature = "selftest")]
static STACK_MAX_USED: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static STACK_MAX_LEAF: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static STACK_CANDIDATES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static STACK_MEASUREMENTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static STACK_MEASUREMENT_VALID: AtomicBool = AtomicBool::new(true);

#[cfg(feature = "selftest")]
pub(super) fn count(observation: &AtomicUsize) {
    assert!(
        observation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .is_ok(),
        "an NVMe self-test observation exhausted"
    );
}

/// The boot stack painted and the register-access leaf armed for one storage candidate.
#[cfg(feature = "selftest")]
#[derive(Debug)]
pub(crate) struct CandidateStackMeasurement {
    slot: StackSlot,
    painted: PaintedStack,
}

/// A candidate's stack use: the painted high-water mark and the deepest live leaf.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CandidateStack {
    /// From the stack's top to the lowest word the candidate wrote.
    pub(crate) deepest: StackUsage,
    /// From the stack's top to the lowest pointer a register access observed.
    pub(crate) leaf: StackUsage,
}

#[cfg(feature = "selftest")]
impl CandidateStackMeasurement {
    /// Paint the dead boot stack below this frame and arm the leaf meter.
    pub(crate) fn begin() -> Option<Self> {
        count(&STACK_CANDIDATES);
        let slot = thread::boot_stack_layout()?.slot(0)?;
        let pointer = cpu::stack_pointer();
        slot.usage_at(pointer)?;
        STACK_METER_STATE
            .compare_exchange(
                STACK_METER_IDLE,
                STACK_METER_PREPARING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        let Some(painted) = paging::paint_current_permanent_stack_for_test() else {
            STACK_METER_STATE.store(STACK_METER_IDLE, Ordering::Release);
            return None;
        };
        STACK_LOWEST.store(pointer, Ordering::Relaxed);
        STACK_METER_STATE.store(STACK_METER_ACTIVE, Ordering::Release);
        Some(Self { slot, painted })
    }

    fn finish(&self) -> Option<CandidateStack> {
        STACK_METER_STATE
            .compare_exchange(
                STACK_METER_ACTIVE,
                STACK_METER_FINISHING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        let leaf = self.slot.usage_at(STACK_LOWEST.load(Ordering::Acquire));
        let deepest = self
            .painted
            .deepest_touch()
            .and_then(|touch| self.slot.usage_at(touch));
        STACK_METER_STATE.store(STACK_METER_IDLE, Ordering::Release);
        let (leaf, deepest) = (leaf?, deepest?);
        // Each leaf pointer names a word its own call wrote, so a shallower scan missed one.
        (deepest.used() >= leaf.used()).then_some(CandidateStack { deepest, leaf })
    }
}

/// Scan one candidate's paint and keep its use, or invalidate the meter if either failed.
#[cfg(feature = "selftest")]
pub(crate) fn finish_candidate_stack(measurement: Option<&CandidateStackMeasurement>) {
    let Some(stack) = measurement.and_then(CandidateStackMeasurement::finish) else {
        STACK_MEASUREMENT_VALID.store(false, Ordering::Release);
        return;
    };
    STACK_MAX_USED.fetch_max(stack.deepest.used(), Ordering::AcqRel);
    STACK_MAX_LEAF.fetch_max(stack.leaf.used(), Ordering::AcqRel);
    count(&STACK_MEASUREMENTS);
}

#[cfg(feature = "selftest")]
#[inline(never)]
pub(super) fn observe_candidate_stack() {
    if STACK_METER_STATE
        .compare_exchange(
            STACK_METER_ACTIVE,
            STACK_METER_OBSERVING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }
    STACK_LOWEST.fetch_min(cpu::stack_pointer(), Ordering::AcqRel);
    STACK_METER_STATE.store(STACK_METER_ACTIVE, Ordering::Release);
}

/// The deepest stack use across every measured candidate, once each one was measured.
#[cfg(feature = "selftest")]
pub(crate) fn candidate_stack_usage_for_test() -> Option<CandidateStack> {
    let candidates = STACK_CANDIDATES.load(Ordering::Acquire);
    if !STACK_MEASUREMENT_VALID.load(Ordering::Acquire)
        || candidates == 0
        || STACK_MEASUREMENTS.load(Ordering::Acquire) != candidates
    {
        return None;
    }
    let slot = thread::boot_stack_layout()?.slot(0)?;
    let usage = |used: u64| {
        slot.stack_top()
            .checked_sub(used)
            .and_then(|pointer| slot.usage_at(pointer))
    };
    Some(CandidateStack {
        deepest: usage(STACK_MAX_USED.load(Ordering::Acquire))?,
        leaf: usage(STACK_MAX_LEAF.load(Ordering::Acquire))?,
    })
}

#[cfg(feature = "selftest")]
pub(crate) fn bus_master_followed_disable_for_test() -> bool {
    let cc = BME_PRECEDING_CC.load(Ordering::Acquire);
    let csts = BME_PRECEDING_CSTS.load(Ordering::Acquire);
    cc != usize::MAX
        && csts != usize::MAX
        && cc & configuration::ENABLE as usize == 0
        && csts & status::READY as usize == 0
}

#[cfg(feature = "selftest")]
pub(crate) fn refused_bus_master_followed_disable_for_test() -> bool {
    let cc = REFUSED_BME_PRECEDING_CC.load(Ordering::Acquire);
    let csts = REFUSED_BME_PRECEDING_CSTS.load(Ordering::Acquire);
    cc != usize::MAX
        && csts != usize::MAX
        && cc & configuration::ENABLE as usize == 0
        && csts & status::READY as usize == 0
}

#[cfg(feature = "selftest")]
pub(crate) fn activity_for_test() -> (usize, usize) {
    (
        START_ATTEMPTS.load(Ordering::Acquire),
        SUBMISSION_DOORBELLS.load(Ordering::Acquire),
    )
}

#[cfg(feature = "selftest")]
pub(super) static HOLD_COMMAND: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static COMMAND_HELD: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(crate) fn command_count_for_test() -> Option<u32> {
    CONTROLLER
        .lock()
        .as_ref()
        .map(|(controller, _)| controller.commands())
}

#[cfg(feature = "selftest")]
pub(crate) fn controller_facts_hold_for_test() -> bool {
    let held = CONTROLLER.lock();
    let Some((controller, _)) = held.as_ref() else {
        return false;
    };
    let capabilities = controller.capabilities();
    capabilities.supports_nvm_command_set()
        && capabilities.supports_page_size(PAGE_SIZE)
        && controller.queue_entries() <= capabilities.max_queue_entries()
        && controller.queue_entries() >= 2
        && controller.has_io_queue()
        && controller.blocks > 0
        && controller.block_bytes.is_power_of_two()
        && controller.block_bytes >= 512
}

#[cfg(feature = "selftest")]
pub(crate) fn malformed_namespace_is_bounded_for_test() -> bool {
    let mut held = CONTROLLER.lock();
    held.as_mut()
        .is_some_and(|(controller, _)| controller.malformed_namespace_stops_before_io_queues())
}

#[cfg(feature = "selftest")]
pub(crate) fn malformed_reads_are_bounded_for_test() -> bool {
    let mut held = CONTROLLER.lock();
    let Some((controller, buffer)) = held.as_mut() else {
        return false;
    };
    controller.malformed_reads_stop_before_submission(buffer)
}

#[cfg(feature = "selftest")]
pub(crate) fn read_block_probe_for_test(lba: u64) -> Option<(usize, [u8; 8], [u8; 8])> {
    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut()?;
    let block = controller.block_bytes as usize;
    if !buffer.fill(0) || controller.read_blocks(lba, 1, buffer).is_err() {
        return None;
    }
    let mut head = [0u8; 8];
    let mut beyond = [0u8; 8];
    if !buffer.read_at(0, &mut head) || !buffer.read_at(block, &mut beyond) {
        return None;
    }
    Some((block, head, beyond))
}

#[cfg(feature = "selftest")]
pub(crate) fn queue_entries_for_test() -> Option<u32> {
    CONTROLLER
        .lock()
        .as_ref()
        .map(|(controller, _)| controller.queue_entries())
}

#[cfg(feature = "selftest")]
pub(crate) fn controller_window_for_test() -> Option<u64> {
    CONTROLLER
        .lock()
        .as_ref()
        .map(|(controller, _)| controller.window())
}

#[cfg(feature = "selftest")]
pub(crate) fn controller_is_unlocked_for_test() -> bool {
    CONTROLLER.try_lock().is_some()
}

#[cfg(feature = "selftest")]
pub(crate) fn timeout_quiesces_for_test() -> bool {
    let mut held = CONTROLLER.lock();
    let Some((controller, buffer)) = held.as_mut() else {
        return false;
    };
    controller.a_timeout_quiesces_before_dma_returns(buffer)
}

#[cfg(feature = "selftest")]
pub(crate) fn measure_buffered_read_for_test() -> Option<(u64, u64)> {
    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut()?;
    controller.measure_buffered_read(buffer)
}

/// Whether the installed domain mints a proof and the unit walks the kernel's root table.
#[cfg(feature = "selftest")]
pub(crate) fn isolation_is_active_for_test() -> Option<(bool, bool)> {
    let held = CONTROLLER.lock();
    held.as_ref()?;
    let installed = crate::iommu::installed();
    let isolation = installed.as_ref()?;
    Some((
        isolation.domain().is_some(),
        isolation.walks_kernel_root_for_test()?,
    ))
}

/// The pages the domain's tables reach beside the frames of every live queue, both sorted.
#[cfg(feature = "selftest")]
pub(crate) fn reachable_frames_for_test() -> Option<(alloc::vec::Vec<u64>, alloc::vec::Vec<u64>)> {
    let held = CONTROLLER.lock();
    let (controller, _) = held.as_ref()?;
    let mut expected = controller.queue_frames()?;
    expected.sort_unstable();
    let installed = crate::iommu::installed();
    let mut reachable = installed.as_ref()?.reachable_pages_for_test()?;
    reachable.sort_unstable();
    Some((reachable, expected))
}

/// Ask the production proof door about the live unit, adding no self-test translation path.
#[cfg(feature = "selftest")]
pub(crate) fn domain_proof_follows_translation_for_test() -> bool {
    let held = CONTROLLER.lock();
    if held.is_none() {
        return false;
    }
    let installed = crate::iommu::installed();
    let Some(isolation) = installed.as_ref() else {
        return false;
    };
    isolation.domain().is_some()
}

/// What a device write into a buffer the domain does not map produced.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FenceWitness {
    /// The command completed or was refused rather than hanging.
    pub(crate) completed: bool,
    /// The priming read's live unmap completed a read-and-write DMA drain.
    pub(crate) drained: bool,
    /// Every byte of the buffer still holds the pattern written before.
    pub(crate) unchanged: bool,
    /// The unit recorded a write from the storage function at the buffer's page.
    pub(crate) faulted: bool,
}

/// Publish a read into the kept buffer unmapped, then ask the unit and the buffer what happened.
/// A priming read maps and unmaps the frame first, so a unit whose cache the unmap did not
/// invalidate would still translate it and let the device write through.
#[cfg(feature = "selftest")]
pub(crate) fn unmapped_write_is_fenced_for_test() -> Option<FenceWitness> {
    const PATTERN: u8 = 0xA5;

    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut()?;
    let drains_before = crate::iommu::installed()
        .as_ref()?
        .completed_dma_drains_for_test();
    controller.read_blocks(0, 1, buffer).ok()?;
    let (source, drained) = {
        let mut installed = crate::iommu::installed();
        let isolation = installed.as_mut()?;
        let drained = isolation.completed_dma_drains_for_test() > drains_before;
        isolation.drain_faults();
        let function = isolation.function()?;
        (
            (u16::from(function.bus()) << 8)
                | (u16::from(function.device()) << 3)
                | u16::from(function.function()),
            drained,
        )
    };
    if !buffer.fill(PATTERN) {
        return None;
    }
    let page = buffer.address()? & !(PAGE_SIZE - 1);

    controller.leave_data_unmapped = true;
    let result = controller.read_blocks(0, 1, buffer);
    controller.leave_data_unmapped = false;

    let mut bytes = alloc::vec![0u8; controller.block_bytes as usize];
    if !buffer.read_at(0, &mut bytes) {
        return None;
    }
    let mut faulted = false;
    let mut installed = crate::iommu::installed();
    while let Some(fault) = installed.as_mut()?.take_fault() {
        faulted |= fault.source == source && fault.address == page && fault.write;
    }
    Some(FenceWitness {
        completed: matches!(result, Ok(()) | Err(BringUpError::Command(_))),
        drained,
        unchanged: bytes.iter().all(|byte| *byte == PATTERN),
        faulted,
    })
}

/// Hold the next submitted command inside its preemptible polling owner.
#[cfg(feature = "selftest")]
pub(crate) fn hold_command_for_test(held: bool) {
    HOLD_COMMAND.store(held, Ordering::Release);
}

/// Whether the test hook still asks a submitted command to remain held.
#[cfg(feature = "selftest")]
pub(crate) fn hold_requested_for_test() -> bool {
    HOLD_COMMAND.load(Ordering::Acquire)
}

/// Whether a submitted command is currently stopped by the test hook.
#[cfg(feature = "selftest")]
pub(crate) fn command_is_held_for_test() -> bool {
    COMMAND_HELD.load(Ordering::Acquire)
}
