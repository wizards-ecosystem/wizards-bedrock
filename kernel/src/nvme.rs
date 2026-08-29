//! An NVMe controller, polled.
//!
//! `libs/nvme` owns every number; `wost` owns the memory-mapped accesses and
//! the frames. What is here is the sequence: bring a controller up, ask it what
//! it is, build one I/O queue pair, and read a block.
//!
//! **Polled is what makes this affordable** (`012` §2). `Create I/O Completion
//! Queue` with `IEN = 0` disables interrupts for that queue, which removes
//! "this kernel has no interrupt sharing" from the critical path entirely. The
//! cost is that a read spins. D-200 confines that polling to the one file
//! worker, where timer and input interrupts may preempt it; `RingEnter` only
//! admits the request. An interrupt-driven controller remains future work.
//!
//! **Three things decide whether this works at all, and none is optional.**
//! Bus master must be on or the controller cannot DMA and *there is no error
//! anywhere to read* — `pci::claim` sets it. The register window must be mapped
//! **uncacheable**; write-back is a silent, hard-to-debug bug. And `CAP` is
//! read, never assumed: the queue size, the doorbell stride, the timeout and
//! the page-size range all come from it, and `012` §11 records that VMware's
//! values are unmeasured.
//!
//! **The ring parallel is a trap.** This looks like `kernel::queue` and is not:
//! there is no completion tail to read, entries are found by their phase tag,
//! completions are matched by command identifier rather than by slot, and
//! capacity is `size - 1`. The arithmetic is shared through `libs/nvme`; the
//! structure is not (D-169).

use nvme::{
    Capabilities, Capacity, Command, Completion, DmaState, NAMESPACE_IDENTIFY_BYTES, Namespace,
    NamespaceError, PollLimit, ReadTransfer, TransferError, configuration, register, status,
};
#[cfg(feature = "selftest")]
use wost::arch::current::cpu;
use wost::arch::current::paging::{DeviceMapping, ReleasableDevice};
use wost::mm::{self, DmaSegment};
use wost::sync::ThreadLock;
use x86_paging::PAGE_SIZE;

use crate::thread::ThreadContext;
use crate::{info, mem, warn};

#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, Ordering};

/// Entries in the queues this driver builds.
///
/// Small: one command is outstanding at a time while the path is polled, and a
/// deeper queue would be depth nothing uses. It is clamped against `CAP.MQES`
/// rather than assumed, because a controller may offer fewer.
const WANTED_ENTRIES: u32 = 8;

/// The identifier of the one I/O queue pair.
const IO_QUEUE: u16 = 1;

/// The namespace this driver reads from.
///
/// One, which is the first and the only one QEMU's controller presents. A
/// driver that enumerated namespaces would ask `Identify` for the list; this
/// one reads the namespace it was told about and says so.
const NAMESPACE: u32 = 1;

/// How many completion reads boot may attempt before the clock is calibrated.
///
/// Once calibrated, the command deadline is real time instead. The attempt
/// bound keeps early bring-up finite before that service exists (D-200).
const POLL_ATTEMPTS: u32 = 5_000_000;

/// Real-time bound for one command once the monotonic clock exists.
const COMMAND_TIMEOUT_MS: u64 = 5_000;

/// One submission queue and the completion queue that answers it.
#[derive(Debug)]
struct QueuePair {
    /// Which queue this is; zero is the admin pair.
    id: u16,
    /// How many entries each holds.
    capacity: Capacity,
    /// The submission entries, physically contiguous.
    submission: DmaSegment,
    /// The completion entries, likewise.
    completion: DmaSegment,
    /// Where the host will write next. **Shadowed**, because the doorbell is
    /// write-only and the controller's idea of the tail cannot be read back.
    tail: u32,
    /// Where the host has consumed up to.
    head: u32,
    /// The phase a *new* completion carries.
    ///
    /// There is no tail to read: an entry is new when its phase tag differs
    /// from the last pass's. A zeroed queue starts at zero, so the first pass
    /// looks for one.
    phase: bool,
}

impl QueuePair {
    /// Allocate and clear both queues.
    fn new(id: u16, capacity: Capacity) -> Option<Self> {
        let submission_frames = frames_for(capacity.submission_bytes());
        let completion_frames = frames_for(capacity.completion_bytes());

        let mut submission = DmaSegment::from(mm::allocate_contiguous(submission_frames, 1)?);
        let mut completion = DmaSegment::from(mm::allocate_contiguous(completion_frames, 1)?);
        // Cleared, because the phase tag of an unwritten entry is what says
        // "nothing here yet", and stale memory says whatever it last held.
        if !submission.zero() || !completion.zero() {
            return None;
        }

        Some(Self {
            id,
            capacity,
            submission,
            completion,
            tail: 0,
            head: 0,
            phase: true,
        })
    }

    /// Withhold every queue frame a controller may still reach.
    fn retain(&mut self) {
        self.submission.retain();
        self.completion.retain();
    }
}

/// Frames enough to hold `bytes`.
const fn frames_for(bytes: u64) -> usize {
    bytes.div_ceil(PAGE_SIZE) as usize
}

/// A controller brought up and ready to be asked for blocks.
#[derive(Debug)]
pub(crate) struct Controller {
    /// Where the registers are readable. Kept so a check can ask **how they
    /// were mapped**: a register file mapped write-back is a driver whose
    /// writes arrive when the cache feels like it, and nothing about the
    /// driver's behaviour would say so on an emulator.
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    window: u64,
    registers: Option<ReleasableDevice>,
    capabilities: Capabilities,
    admin: QueuePair,
    io: Option<QueuePair>,
    /// The next command identifier. Wrapping is fine: a completion is matched
    /// by it and only one command is outstanding at a time.
    next_id: u16,
    /// Whether the one outstanding command's memory is host-owned again.
    dma: DmaState,
    /// Makes the production timeout arm deterministic for its machine proof.
    #[cfg(feature = "selftest")]
    force_timeout: bool,
    /// Stop a validated read at the last seam before command construction.
    #[cfg(feature = "selftest")]
    intercept_read_submission: bool,
    /// Invalid requests which crossed validation during the current witness.
    #[cfg(feature = "selftest")]
    intercepted_read_submissions: u32,
    /// Stop I/O queue creation before allocation or device publication.
    #[cfg(feature = "selftest")]
    intercept_io_queue_creation: bool,
    /// Queue creations which crossed namespace validation during the witness.
    #[cfg(feature = "selftest")]
    intercepted_io_queue_creations: u32,
    /// What `Identify` said, kept so a check can ask rather than infer.
    pub(crate) blocks: u64,
    /// Bytes in one logical block.
    pub(crate) block_bytes: u32,
}

/// Why a controller could not be brought up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BringUpError {
    /// The register window could not be mapped.
    NotMapped,
    /// The window was mapped and could not be claimed, which means the space
    /// it was mapped into is not the one running.
    NotClaimable,
    /// The controller reports no NVM command set, so it has no blocks.
    NoCommandSet,
    /// The controller cannot work in this kernel's page size.
    PageSize,
    /// `CAP.MQES` describes a queue that holds nothing.
    QueueSize,
    /// No memory for the queues.
    NoMemory,
    /// The controller did not become ready, or gave up.
    NotReady,
    /// A command was refused, or never answered.
    Command(u16),
    /// A read did not fit its command, namespace or DMA owner.
    InvalidTransfer(TransferError),
    /// Identify Namespace data did not fit the supported storage contract.
    InvalidNamespace(NamespaceError),
}

impl core::fmt::Display for BringUpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotMapped => f.write_str("the register window could not be mapped"),
            Self::NotClaimable => f.write_str("the register window is not in the running space"),
            Self::NoCommandSet => f.write_str("no nvm command set"),
            Self::PageSize => f.write_str("this kernel's page size is outside mpsmin..mpsmax"),
            Self::QueueSize => f.write_str("the reported queue size holds nothing"),
            Self::NoMemory => f.write_str("no memory for the queues"),
            Self::NotReady => f.write_str("the controller never became ready"),
            Self::Command(status) => write!(f, "a command was refused: status {status:#x}"),
            Self::InvalidTransfer(reason) => write!(f, "an invalid transfer: {reason}"),
            Self::InvalidNamespace(reason) => write!(f, "an invalid namespace: {reason}"),
        }
    }
}

/// A failed candidate and whether every resource it touched was retired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StartFailure {
    reason: BringUpError,
    retired: bool,
}

impl StartFailure {
    /// Why bring-up failed.
    pub(crate) const fn reason(self) -> BringUpError {
        self.reason
    }

    /// Whether selection may safely advance to another controller.
    pub(crate) const fn is_retired(self) -> bool {
        self.retired
    }
}

/// Bring a controller up on the registers at `base`.
///
/// The specification's initialisation sequence, in order, with the two waits
/// bounded by `CAP.TO` rather than by a number chosen here.
pub(crate) fn bring_up(
    context: &mut ThreadContext,
    base: u64,
    len: u64,
) -> Result<Controller, StartFailure> {
    // Uncacheable, which `map_device` chooses: a register file mapped
    // write-back is a driver whose writes arrive when the cache feels like it.
    let mapping: DeviceMapping = mem::map_device(context, base, len)
        .and_then(Result::ok)
        .ok_or(StartFailure {
            reason: BringUpError::NotMapped,
            retired: true,
        })?;
    let window = mapping.address();
    let mut registers = mapping.claim_releasable().map_err(|_| StartFailure {
        reason: BringUpError::NotClaimable,
        retired: false,
    })?;

    // `CAP` in two halves, because an `MmioRegion` makes thirty-two-bit
    // accesses. The specification allows it for this register.
    let low = read(&registers, register::CAP).map_err(|reason| StartFailure {
        reason,
        retired: false,
    })?;
    let high = read(&registers, register::CAP + 4).map_err(|reason| StartFailure {
        reason,
        retired: false,
    })?;
    let capabilities = Capabilities::from_bits(u64::from(low) | (u64::from(high) << 32));

    // Disabled before any policy refusal, so even an unsupported controller
    // can be retired without leaving firmware DMA behind.
    if let Err(reason) = write(&mut registers, register::CC, 0)
        .and_then(|()| wait_for_ready(&mut registers, capabilities, false))
    {
        return Err(StartFailure {
            reason,
            retired: false,
        });
    }

    if !capabilities.supports_nvm_command_set() {
        return Err(retire_disabled(
            context,
            registers,
            BringUpError::NoCommandSet,
        ));
    }
    if !capabilities.supports_page_size(PAGE_SIZE) {
        return Err(retire_disabled(context, registers, BringUpError::PageSize));
    }
    let Some(most) = Capacity::new(capabilities.max_queue_entries()) else {
        return Err(retire_disabled(context, registers, BringUpError::QueueSize));
    };
    let Some(capacity) = Capacity::new(WANTED_ENTRIES.min(most.entries())) else {
        return Err(retire_disabled(context, registers, BringUpError::QueueSize));
    };

    let Some(admin) = QueuePair::new(0, capacity) else {
        return Err(retire_disabled(context, registers, BringUpError::NoMemory));
    };
    let Some(submission) = admin.submission.address() else {
        return Err(retire_disabled(context, registers, BringUpError::NoMemory));
    };
    let Some(completion) = admin.completion.address() else {
        return Err(retire_disabled(context, registers, BringUpError::NoMemory));
    };
    let aqa = (capacity.as_zero_based() << 16) | capacity.as_zero_based();
    if let Err(reason) = write(&mut registers, register::AQA, aqa)
        .and_then(|()| write_u64(&mut registers, register::ASQ, submission))
        .and_then(|()| write_u64(&mut registers, register::ACQ, completion))
    {
        return Err(retire_disabled(context, registers, reason));
    }

    // `MPS` is the host page size as `log2 - 12`, and the entry sizes are the
    // specification's fixed logarithms.
    let page_shift = PAGE_SIZE.trailing_zeros() - 12;
    let cc = configuration::ENABLE
        | (page_shift << configuration::PAGE_SIZE_SHIFT)
        | (nvme::IOSQES << configuration::SUBMISSION_ENTRY_SHIFT)
        | (nvme::IOCQES << configuration::COMPLETION_ENTRY_SHIFT);

    // Cleanup owns every published queue address before the enable write.
    let mut controller = Controller {
        window,
        registers: Some(registers),
        capabilities,
        admin,
        io: None,
        next_id: 1,
        dma: DmaState::new(),
        #[cfg(feature = "selftest")]
        force_timeout: false,
        #[cfg(feature = "selftest")]
        intercept_read_submission: false,
        #[cfg(feature = "selftest")]
        intercepted_read_submissions: 0,
        #[cfg(feature = "selftest")]
        intercept_io_queue_creation: false,
        #[cfg(feature = "selftest")]
        intercepted_io_queue_creations: 0,
        blocks: 0,
        block_bytes: 0,
    };

    if let Err(reason) = controller
        .registers
        .as_mut()
        .ok_or(BringUpError::NotMapped)
        .and_then(|registers| write(registers, register::CC, cc))
        .and_then(|()| {
            let registers = controller
                .registers
                .as_mut()
                .ok_or(BringUpError::NotMapped)?;
            wait_for_ready(registers, capabilities, true)
        })
    {
        let retired = controller.retire(context);
        return Err(StartFailure { reason, retired });
    }

    if let Err(reason) = controller.identify_namespace(capacity) {
        let retired = controller.retire(context);
        return Err(StartFailure { reason, retired });
    }
    Ok(controller)
}

/// Return a mapping while the controller is already verified disabled.
fn retire_disabled(
    context: &mut ThreadContext,
    registers: ReleasableDevice,
    reason: BringUpError,
) -> StartFailure {
    let retired = matches!(mem::unmap_device(context, registers), Some(Ok(())));
    StartFailure { reason, retired }
}

impl Controller {
    /// Commands this controller has been given.
    ///
    /// The identifier counter, which advances once per command submitted. It
    /// is what makes "the device was not asked" observable from outside the
    /// driver — a refusal that happened *before* the read reached the
    /// controller looks exactly like one that happened after, unless something
    /// counts (D-173).
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) const fn commands(&self) -> u16 {
        self.next_id
    }

    /// Where the register window is readable.
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) const fn window(&self) -> u64 {
        self.window
    }

    /// What `CAP` said.
    pub(crate) const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Entries in each queue.
    pub(crate) const fn queue_entries(&self) -> u32 {
        self.admin.capacity.entries()
    }

    /// Whether an I/O queue pair was built.
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) const fn has_io_queue(&self) -> bool {
        self.io.is_some()
    }

    /// Ask the namespace how large it is and how large its blocks are.
    fn identify_namespace(&mut self, capacity: Capacity) -> Result<(), BringUpError> {
        let mut buffer =
            DmaSegment::from(mm::allocate_contiguous(1, 1).ok_or(BringUpError::NoMemory)?);
        if !buffer.zero() {
            return Err(BringUpError::NoMemory);
        }
        let address = buffer.address().ok_or(BringUpError::NoMemory)?;

        // CNS = 0: this namespace's data structure.
        let command = Command::new(nvme::admin::IDENTIFY, self.take_id())
            .namespace(NAMESPACE)
            .buffers(address, 0)
            .dword(10, 0);
        self.run_admin(command, Some(&mut buffer))?;

        let mut identify = [0u8; NAMESPACE_IDENTIFY_BYTES];
        if !buffer.read_at(0, &mut identify) {
            return Err(BringUpError::NoMemory);
        }
        self.configure_namespace(&identify, capacity)
    }

    /// Validate and commit namespace geometry before I/O queues can exist.
    fn configure_namespace(
        &mut self,
        identify: &[u8; NAMESPACE_IDENTIFY_BYTES],
        capacity: Capacity,
    ) -> Result<(), BringUpError> {
        let namespace = Namespace::parse(identify).map_err(BringUpError::InvalidNamespace)?;
        self.create_io_queues(capacity)?;
        self.blocks = namespace.blocks();
        self.block_bytes = namespace.block_bytes();
        Ok(())
    }

    /// Build the one I/O queue pair, with its interrupts disabled.
    fn create_io_queues(&mut self, capacity: Capacity) -> Result<(), BringUpError> {
        #[cfg(feature = "selftest")]
        if self.intercept_io_queue_creation {
            self.intercepted_io_queue_creations += 1;
            return Err(BringUpError::NotReady);
        }

        let pair = QueuePair::new(IO_QUEUE, capacity).ok_or(BringUpError::NoMemory)?;
        let completion_address = pair.completion.address().ok_or(BringUpError::NoMemory)?;
        let submission_address = pair.submission.address().ok_or(BringUpError::NoMemory)?;
        // Once either command is published, the controller can retain these
        // runs. Keeping them here gives failure cleanup ownership of both.
        self.io = Some(pair);

        // The completion queue first: a submission queue names the completion
        // queue that answers it, so the answer has to exist.
        //
        // `CDW11` bit 0 is `PC`, physically contiguous, which these are. Bit 1
        // is `IEN`, and it is **left clear**: that is what makes this polled
        // and what removes interrupt sharing from the critical path (`012` §2).
        let completion = Command::new(nvme::admin::CREATE_COMPLETION_QUEUE, self.take_id())
            .buffers(completion_address, 0)
            .dword(10, (capacity.as_zero_based() << 16) | u32::from(IO_QUEUE))
            .dword(11, 1);
        self.run_admin(completion, None)?;

        // `CDW11` here is the completion queue's identifier in the high half,
        // and `PC` in bit 0.
        let submission = Command::new(nvme::admin::CREATE_SUBMISSION_QUEUE, self.take_id())
            .buffers(submission_address, 0)
            .dword(10, (capacity.as_zero_based() << 16) | u32::from(IO_QUEUE))
            .dword(11, (u32::from(IO_QUEUE) << 16) | 1);
        self.run_admin(submission, None)?;

        Ok(())
    }

    /// Read `blocks` logical blocks starting at `lba` into `into`.
    ///
    /// # Errors
    ///
    /// [`BringUpError::InvalidTransfer`] before submission when the command,
    /// namespace or DMA extent does not fit; otherwise the controller's
    /// command, memory and readiness errors.
    pub(crate) fn read_blocks(
        &mut self,
        lba: u64,
        blocks: u32,
        into: &mut DmaSegment,
    ) -> Result<(), BringUpError> {
        let buffer_bytes = into.len().ok_or(BringUpError::NotReady)?;
        let request = self.validate_read(lba, blocks, buffer_bytes)?;
        self.submit_read(request, into)
    }

    /// Measure one read which fills the retained DMA buffer.
    #[cfg(feature = "selftest")]
    pub(crate) fn measure_buffered_read(&mut self, into: &mut DmaSegment) -> Option<(u64, u64)> {
        let buffer_bytes = into.len()?;
        let block_bytes = u64::from(self.block_bytes);
        if block_bytes == 0 {
            return None;
        }
        let blocks = u32::try_from(
            (buffer_bytes / block_bytes)
                .min(self.blocks)
                .min(u64::from(nvme::MAX_TRANSFER_BLOCKS)),
        )
        .ok()?;
        if blocks == 0 {
            return None;
        }
        let bytes = u64::from(blocks).checked_mul(block_bytes)?;
        let started = cpu::timestamp();
        self.read_blocks(0, blocks, into).ok()?;
        let ticks = cpu::timestamp().checked_sub(started)?;
        (ticks > 0).then_some((bytes, ticks))
    }

    /// Bind raw caller arguments to this namespace before submission.
    fn validate_read(
        &self,
        lba: u64,
        blocks: u32,
        buffer_bytes: u64,
    ) -> Result<ReadTransfer, BringUpError> {
        ReadTransfer::new(
            lba,
            blocks,
            self.blocks,
            u64::from(self.block_bytes),
            buffer_bytes,
        )
        .map_err(BringUpError::InvalidTransfer)
    }

    /// Build and publish a read already proved to fit every outer bound.
    fn submit_read(
        &mut self,
        request: ReadTransfer,
        into: &mut DmaSegment,
    ) -> Result<(), BringUpError> {
        #[cfg(feature = "selftest")]
        if self.intercept_read_submission {
            self.intercepted_read_submissions += 1;
            return Err(BringUpError::NotReady);
        }

        if self.io.is_none() {
            return Err(BringUpError::NotReady);
        }
        let address = into.address().ok_or(BringUpError::NotReady)?;
        let built = nvme::regions(address, request.bytes(), PAGE_SIZE, None)
            .ok_or(BringUpError::NoMemory)?;
        let lba = request.lba();

        let command = Command::new(nvme::nvm::READ, self.take_id())
            .namespace(NAMESPACE)
            .buffers(built.prp1, built.prp2)
            .dword(10, lba as u32)
            .dword(11, (lba >> 32) as u32)
            // **Zero-based**, like every count in this specification: one block
            // is a zero here, and an off-by-one reads a block too many.
            .dword(12, u32::from(request.zero_based_blocks()));

        self.run_on(IO_QUEUE, command, Some(into))
    }

    /// Prove malformed Identify data cannot reach I/O queue creation.
    #[cfg(feature = "selftest")]
    pub(crate) fn malformed_namespace_stops_before_io_queues(&mut self) -> bool {
        const SIZE: usize = 0;
        const FORMAT_COUNT: usize = 25;
        const FORMATTED: usize = 26;
        const PROTECTION: usize = 29;
        const FIRST_FORMAT: usize = 128;
        const METADATA: usize = FIRST_FORMAT;
        const BLOCK_SHIFT: usize = FIRST_FORMAT + 2;
        const FORMAT_CONTROL: usize = FIRST_FORMAT + 3;

        let capacity = self.admin.capacity;
        let blocks_before = self.blocks;
        let block_bytes_before = self.block_bytes;
        let commands_before = self.commands();
        let queue_before = self.has_io_queue();
        let intercepted_before = self.intercepted_io_queue_creations;

        let mut valid = [0u8; NAMESPACE_IDENTIFY_BYTES];
        valid[SIZE] = 1;
        valid[BLOCK_SHIFT] = nvme::SUPPORTED_BLOCK_BYTES.trailing_zeros() as u8;

        self.intercept_io_queue_creation = true;

        let mut empty = valid;
        empty[SIZE] = 0;
        let empty = self.configure_namespace(&empty, capacity);

        let mut formats = valid;
        formats[FORMAT_COUNT] = 16;
        let formats = self.configure_namespace(&formats, capacity);

        let mut outside = valid;
        outside[FORMATTED] = 1;
        let outside = self.configure_namespace(&outside, capacity);

        let mut reserved_selector = valid;
        reserved_selector[FORMATTED] = 0x80;
        let reserved_selector = self.configure_namespace(&reserved_selector, capacity);

        let mut reserved_format = valid;
        reserved_format[FORMAT_CONTROL] = 4;
        let reserved_format = self.configure_namespace(&reserved_format, capacity);

        let mut metadata = valid;
        metadata[METADATA] = 1;
        let metadata = self.configure_namespace(&metadata, capacity);

        let mut protection = valid;
        protection[PROTECTION] = 1;
        let protection = self.configure_namespace(&protection, capacity);

        let mut block_size = valid;
        block_size[BLOCK_SHIFT] = 32;
        let block_size = self.configure_namespace(&block_size, capacity);

        self.intercept_io_queue_creation = false;

        let refused = [
            (empty, NamespaceError::Empty),
            (formats, NamespaceError::TooManyFormats(17)),
            (
                outside,
                NamespaceError::FormatOutsideList {
                    selected: 1,
                    formats: 1,
                },
            ),
            (
                reserved_selector,
                NamespaceError::ReservedFormatBits(0x80),
            ),
            (
                reserved_format,
                NamespaceError::ReservedLbaFormatBits(4),
            ),
            (metadata, NamespaceError::Metadata(1)),
            (protection, NamespaceError::Protection(1)),
            (block_size, NamespaceError::BlockSize(32)),
        ]
        .into_iter()
        .all(|(result, expected)| {
            matches!(result, Err(BringUpError::InvalidNamespace(found)) if found == expected)
        });

        refused
            && self.blocks == blocks_before
            && self.block_bytes == block_bytes_before
            && self.commands() == commands_before
            && self.has_io_queue() == queue_before
            && self.intercepted_io_queue_creations == intercepted_before
    }

    /// Prove malformed reads stop before the private submission seam.
    #[cfg(feature = "selftest")]
    pub(crate) fn malformed_reads_stop_before_submission(&mut self, into: &mut DmaSegment) -> bool {
        let Some(buffer_bytes) = into.len() else {
            return false;
        };
        let block_bytes = u64::from(self.block_bytes);
        if block_bytes == 0 {
            return false;
        }
        let short_blocks = buffer_bytes / block_bytes + 1;
        let Ok(short_blocks) = u32::try_from(short_blocks) else {
            return false;
        };
        if short_blocks > nvme::MAX_TRANSFER_BLOCKS || u64::from(short_blocks) > self.blocks {
            return false;
        }

        let commands_before = self.commands();
        let intercepted_before = self.intercepted_read_submissions;
        self.intercept_read_submission = true;
        let refused = [
            (
                self.read_blocks(0, 0, into),
                TransferError::Empty,
            ),
            (
                self.read_blocks(0, nvme::MAX_TRANSFER_BLOCKS + 1, into),
                TransferError::TooManyBlocks,
            ),
            (
                self.read_blocks(u64::MAX, 1, into),
                TransferError::OutsideNamespace,
            ),
            (
                self.read_blocks(self.blocks, 1, into),
                TransferError::OutsideNamespace,
            ),
            (
                self.read_blocks(0, short_blocks, into),
                TransferError::BufferTooSmall,
            ),
        ]
        .into_iter()
        .all(|(result, expected)| {
            matches!(result, Err(BringUpError::InvalidTransfer(found)) if found == expected)
        });
        self.intercept_read_submission = false;

        refused
            && self.commands() == commands_before
            && self.intercepted_read_submissions == intercepted_before
    }

    /// The next command identifier.
    fn take_id(&mut self) -> u16 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Run one command on the admin queue.
    fn run_admin(
        &mut self,
        command: Command,
        data: Option<&mut DmaSegment>,
    ) -> Result<(), BringUpError> {
        self.run_on(0, command, data)
    }

    /// Submit `command` to queue `which` and spin until it is answered.
    fn run_on(
        &mut self,
        which: u16,
        command: Command,
        mut data: Option<&mut DmaSegment>,
    ) -> Result<(), BringUpError> {
        let expected = command.id();
        let capabilities = self.capabilities;

        // Written, published, and only then rung: the controller may read the
        // entry the instant the doorbell moves.
        let (submission_tail, submission_doorbell) = {
            let queue = self.queue_mut(which).ok_or(BringUpError::NotReady)?;
            if !queue.capacity.has_room(queue.head, queue.tail) {
                return Err(BringUpError::NotReady);
            }
            let at = queue.tail as usize * nvme::SUBMISSION_ENTRY_BYTES;
            if !queue.submission.write_at(at, &command.to_bytes()) {
                return Err(BringUpError::NoMemory);
            }
            queue.tail = queue.capacity.next(queue.tail);
            (
                queue.tail,
                nvme::doorbell(capabilities, which, false).ok_or(BringUpError::NotReady)?,
            )
        };
        if !self.dma.begin() {
            return Err(BringUpError::NotReady);
        }
        let rung = self
            .registers
            .as_mut()
            .ok_or(BringUpError::NotMapped)
            .and_then(|registers| write(registers, submission_doorbell, submission_tail));
        if let Err(error) = rung {
            return self.fail_submitted(error, data.as_deref_mut());
        }

        #[cfg(feature = "selftest")]
        if HOLD_COMMAND.load(Ordering::Acquire) {
            COMMAND_HELD.store(true, Ordering::Release);
            while HOLD_COMMAND.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
            COMMAND_HELD.store(false, Ordering::Release);
        }

        // Poll for a completion carrying the phase this pass expects. There is
        // no tail to read; the phase tag is the whole of how a new entry is
        // recognised (`012` §3).
        #[cfg(feature = "selftest")]
        if self.force_timeout {
            return self.fail_submitted(BringUpError::NotReady, data);
        }

        let mut limit = PollLimit::after_ms(clock_ns(), COMMAND_TIMEOUT_MS, POLL_ATTEMPTS);
        while limit.permit(clock_ns()) {
            let found = {
                let Some(queue) = self.queue_mut(which) else {
                    return self.fail_submitted(BringUpError::NotReady, data.as_deref_mut());
                };
                let at = queue.head as usize * nvme::COMPLETION_ENTRY_BYTES;
                let mut bytes = [0u8; nvme::COMPLETION_ENTRY_BYTES];
                if !queue.completion.read_at(at, &mut bytes) {
                    return self.fail_submitted(BringUpError::NoMemory, data.as_deref_mut());
                }
                let entry = Completion::from_bytes(bytes);
                if entry.phase == queue.phase {
                    let head = queue.capacity.next(queue.head);
                    // The phase flips on every wrap, which is what lets the
                    // *next* pass tell a new entry from the one still there.
                    if head == 0 {
                        queue.phase = !queue.phase;
                    }
                    queue.head = head;
                    Some(entry)
                } else {
                    None
                }
            };

            if let Some(entry) = found {
                if entry.id != expected {
                    return self
                        .fail_submitted(BringUpError::Command(entry.status), data.as_deref_mut());
                }
                self.dma.complete();

                let Some(doorbell) = nvme::doorbell(capabilities, which, true) else {
                    return self.fail_controller(BringUpError::NotReady);
                };
                let Some(head) = self.queue_mut(which).map(|queue| queue.head) else {
                    return self.fail_controller(BringUpError::NotReady);
                };
                let acknowledged = self
                    .registers
                    .as_mut()
                    .ok_or(BringUpError::NotMapped)
                    .and_then(|registers| write(registers, doorbell, head));
                if let Err(error) = acknowledged {
                    return self.fail_controller(error);
                }
                if !entry.is_success() {
                    return Err(BringUpError::Command(entry.status));
                }
                return Ok(());
            }
            core::hint::spin_loop();
        }
        self.fail_submitted(BringUpError::NotReady, data)
    }

    /// Stop uncertain DMA before exposing an error to the caller.
    fn fail_submitted(
        &mut self,
        error: BringUpError,
        data: Option<&mut DmaSegment>,
    ) -> Result<(), BringUpError> {
        self.stop_dma(data);
        Err(error)
    }

    /// Fail closed after a completion made the transfer buffer safe.
    fn fail_controller(&mut self, error: BringUpError) -> Result<(), BringUpError> {
        self.stop_dma(None);
        Err(error)
    }

    /// Disable the controller and verify it idle, or withhold all DMA memory.
    fn stop_dma(&mut self, data: Option<&mut DmaSegment>) {
        if !self.dma.is_settled() {
            let stopped = self.registers.as_mut().is_some_and(|registers| {
                write(registers, register::CC, 0)
                    .and_then(|()| wait_for_ready(registers, self.capabilities, false))
                    .is_ok()
            });
            self.dma.quiesce(stopped);
        }

        if self.dma.must_retain() {
            self.admin.retain();
            if let Some(queue) = self.io.as_mut() {
                queue.retain();
            }
            if let Some(segment) = data {
                segment.retain();
            }
        }
    }

    /// Stop DMA, consume register access, and return the mapping.
    pub(crate) fn retire(mut self, context: &mut ThreadContext) -> bool {
        self.stop_dma(None);
        if !self.dma.is_quiesced() {
            return false;
        }
        let Some(registers) = self.registers.take() else {
            return false;
        };
        matches!(mem::unmap_device(context, registers), Some(Ok(())))
    }

    /// Force the ordinary timeout arm and prove its cleanup before returning.
    #[cfg(feature = "selftest")]
    pub(crate) fn a_timeout_quiesces_before_dma_returns(&mut self, data: &mut DmaSegment) -> bool {
        self.force_timeout = true;
        let result = self.read_blocks(0, 1, data);
        self.force_timeout = false;

        let protected = matches!(result, Err(BringUpError::NotReady))
            && self.dma.is_quiesced()
            && !data.is_retained();
        // A timeout mutation must turn the check red without letting its
        // deliberately outstanding command make the remainder of the boot
        // unsafe. Repair only after remembering the failed observation.
        if self.dma.is_in_flight() {
            let _ = self.fail_submitted(BringUpError::NotReady, Some(data));
        }
        protected
    }

    /// The queue pair with this identifier.
    fn queue_mut(&mut self, which: u16) -> Option<&mut QueuePair> {
        if which == self.admin.id {
            return Some(&mut self.admin);
        }
        self.io.as_mut().filter(|queue| queue.id == which)
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop_dma(None);
    }
}

/// Spin until `CSTS.RDY` matches `wanted`, bounded by `CAP.TO`.
///
/// `CSTS.CFS` ends an enable wait. During disable, observed `RDY = 0` wins:
/// fatal status does not mean the controller can still reach host memory.
fn wait_for_ready(
    registers: &mut ReleasableDevice,
    capabilities: Capabilities,
    wanted: bool,
) -> Result<(), BringUpError> {
    let timeout_ms = capabilities.timeout_ms().max(500);
    let fallback = (timeout_ms.saturating_mul(20_000)).min(20_000_000) as u32;
    let mut limit = PollLimit::after_ms(clock_ns(), timeout_ms, fallback);
    while limit.permit(clock_ns()) {
        let bits = read(registers, register::CSTS)?;
        if ((bits & status::READY) != 0) == wanted {
            return Ok(());
        }
        if wanted && bits & status::FATAL != 0 {
            return Err(BringUpError::NotReady);
        }
        core::hint::spin_loop();
    }
    Err(BringUpError::NotReady)
}

/// Read the monotonic clock only after calibration has made it meaningful.
fn clock_ns() -> Option<u64> {
    crate::time::is_calibrated().then(crate::time::now_ns)
}

fn read(registers: &ReleasableDevice, offset: usize) -> Result<u32, BringUpError> {
    registers
        .read_u32(offset)
        .map_err(|_| BringUpError::NotMapped)
}

fn write(registers: &mut ReleasableDevice, offset: usize, value: u32) -> Result<(), BringUpError> {
    registers
        .write_u32(offset, value)
        .map_err(|_| BringUpError::NotMapped)
}

/// Write a sixty-four-bit register as two halves, low first.
///
/// The specification allows it for `ASQ` and `ACQ`, and an `MmioRegion` makes
/// thirty-two-bit accesses. Low first because the controller latches on the
/// high half.
fn write_u64(
    registers: &mut ReleasableDevice,
    offset: usize,
    value: u64,
) -> Result<(), BringUpError> {
    write(registers, offset, value as u32)?;
    write(registers, offset + 4, (value >> 32) as u32)
}

/// Bring the controller up and say what it is.
///
/// Reports a retirement-qualified failure: the selector may keep booting
/// without storage, but may try a sibling only when teardown was exact.
pub(crate) fn start(
    context: &mut ThreadContext,
    base: u64,
    len: u64,
) -> Result<Controller, StartFailure> {
    match bring_up(context, base, len) {
        Ok(controller) => {
            info!(
                "nvme: up — {} entries a queue, {} blocks of {} bytes, doorbells {} apart",
                controller.queue_entries(),
                controller.blocks,
                controller.block_bytes,
                controller.capabilities().doorbell_bytes()
            );
            Ok(controller)
        }
        Err(failure) => {
            warn!("nvme: not brought up: {}", failure.reason());
            Err(failure)
        }
    }
}

/// The one controller, kept so anything that wants blocks can ask.
///
/// A `static` rather than a value threaded through boot because the driver
/// outlives the function that built it. Runtime reads reach it only through
/// the file worker (D-200).
static CONTROLLER: ThreadLock<Option<(Controller, DmaSegment)>> = ThreadLock::new(None);

#[cfg(feature = "selftest")]
static HOLD_COMMAND: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static COMMAND_HELD: AtomicBool = AtomicBool::new(false);

/// Keep a controller and the buffer it reads into.
pub(crate) fn keep(controller: Controller, buffer: DmaSegment) {
    *CONTROLLER.lock() = Some((controller, buffer));
}

/// Run `f` against the controller, if there is one.
///
/// The thread lock may remain held while the polled command is preempted. Only
/// the file worker performs runtime reads, and no interrupt handler or second
/// thread contends for the controller (D-200).
pub(crate) fn with_controller<T>(
    f: impl FnOnce(&mut Controller, &mut DmaSegment) -> T,
) -> Option<T> {
    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut()?;
    Some(f(controller, buffer))
}

/// Hold the next submitted command inside its preemptible polling owner.
#[cfg(feature = "selftest")]
pub(crate) fn hold_command_for_test(held: bool) {
    HOLD_COMMAND.store(held, Ordering::Release);
}

/// Whether a submitted command is currently stopped by the test hook.
#[cfg(feature = "selftest")]
pub(crate) fn command_is_held_for_test() -> bool {
    COMMAND_HELD.load(Ordering::Acquire)
}
