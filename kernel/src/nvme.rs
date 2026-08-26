//! An NVMe controller, polled.
//!
//! `libs/nvme` owns every number; `wost` owns the memory-mapped accesses and
//! the frames. What is here is the sequence: bring a controller up, ask it what
//! it is, build one I/O queue pair, and read a block.
//!
//! **Polled is what makes this affordable** (`012` §2). `Create I/O Completion
//! Queue` with `IEN = 0` disables interrupts for that queue, which removes
//! "this kernel has no interrupt sharing" from the critical path entirely. The
//! cost is that a read spins; slice 9 is where that becomes a submission and
//! the spinning becomes a wait.
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

use nvme::{Capabilities, Capacity, Command, Completion, configuration, register, status};
use wost::arch::current::paging::DeviceMapping;
use wost::mm::{self, Segment};
use wost::mmio::MmioRegion;
use wost::sync::SpinLock;
use x86_paging::PAGE_SIZE;

use crate::thread::ThreadContext;
use crate::{info, mem, warn};

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

/// How long to spin for a completion, in reads of the queue.
///
/// Bounded rather than endless: a controller that does not answer is a
/// controller to report, and spinning on it forgives exactly the failure worth
/// knowing about — which on a machine without bus master enabled is the only
/// symptom there is.
const POLL_ATTEMPTS: u32 = 5_000_000;

/// One submission queue and the completion queue that answers it.
#[derive(Debug)]
struct QueuePair {
    /// Which queue this is; zero is the admin pair.
    id: u16,
    /// How many entries each holds.
    capacity: Capacity,
    /// The submission entries, physically contiguous.
    submission: Segment,
    /// The completion entries, likewise.
    completion: Segment,
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

        let mut submission = mm::allocate_contiguous(submission_frames, 1)?;
        let mut completion = mm::allocate_contiguous(completion_frames, 1)?;
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
    registers: MmioRegion,
    capabilities: Capabilities,
    admin: QueuePair,
    io: Option<QueuePair>,
    /// The next command identifier. Wrapping is fine: a completion is matched
    /// by it and only one command is outstanding at a time.
    next_id: u16,
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
        }
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
) -> Result<Controller, BringUpError> {
    // Uncacheable, which `map_device` chooses: a register file mapped
    // write-back is a driver whose writes arrive when the cache feels like it.
    let mapping: DeviceMapping =
        mem::with_active_space(context, |space| space.map_device(base, len))
            .ok_or(BringUpError::NotMapped)?
            .map_err(|_| BringUpError::NotMapped)?;
    let window = mapping.address();
    let mut registers = mapping.claim().ok_or(BringUpError::NotClaimable)?;

    // `CAP` in two halves, because an `MmioRegion` makes thirty-two-bit
    // accesses. The specification allows it for this register.
    let low = read(&registers, register::CAP)?;
    let high = read(&registers, register::CAP + 4)?;
    let capabilities = Capabilities::from_bits(u64::from(low) | (u64::from(high) << 32));

    if !capabilities.supports_nvm_command_set() {
        return Err(BringUpError::NoCommandSet);
    }
    if !capabilities.supports_page_size(PAGE_SIZE) {
        return Err(BringUpError::PageSize);
    }
    let most = Capacity::new(capabilities.max_queue_entries()).ok_or(BringUpError::QueueSize)?;
    let capacity =
        Capacity::new(WANTED_ENTRIES.min(most.entries())).ok_or(BringUpError::QueueSize)?;

    // Disabled first, and waited for: a controller left enabled by firmware
    // ignores every register below.
    write(&mut registers, register::CC, 0)?;
    wait_for_ready(&mut registers, capabilities, false)?;

    let admin = QueuePair::new(0, capacity).ok_or(BringUpError::NoMemory)?;
    let aqa = (capacity.as_zero_based() << 16) | capacity.as_zero_based();
    write(&mut registers, register::AQA, aqa)?;
    write_u64(&mut registers, register::ASQ, admin.submission.address())?;
    write_u64(&mut registers, register::ACQ, admin.completion.address())?;

    // `MPS` is the host page size as `log2 - 12`, and the entry sizes are the
    // specification's fixed logarithms.
    let page_shift = PAGE_SIZE.trailing_zeros() - 12;
    let cc = configuration::ENABLE
        | (page_shift << configuration::PAGE_SIZE_SHIFT)
        | (nvme::IOSQES << configuration::SUBMISSION_ENTRY_SHIFT)
        | (nvme::IOCQES << configuration::COMPLETION_ENTRY_SHIFT);
    write(&mut registers, register::CC, cc)?;
    wait_for_ready(&mut registers, capabilities, true)?;

    let mut controller = Controller {
        window,
        registers,
        capabilities,
        admin,
        io: None,
        next_id: 1,
        blocks: 0,
        block_bytes: 0,
    };

    controller.identify_namespace()?;
    controller.create_io_queues(capacity)?;
    Ok(controller)
}

impl Controller {
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
    fn identify_namespace(&mut self) -> Result<(), BringUpError> {
        let mut buffer = mm::allocate_contiguous(1, 1).ok_or(BringUpError::NoMemory)?;
        if !buffer.zero() {
            return Err(BringUpError::NoMemory);
        }

        // CNS = 0: this namespace's data structure.
        let command = Command::new(nvme::admin::IDENTIFY, self.take_id())
            .namespace(NAMESPACE)
            .buffers(buffer.address(), 0)
            .dword(10, 0);
        self.run_admin(command)?;

        // `NSZE` at 0, and the formatted LBA size index at 26, which selects
        // one of the sixteen `LBAF` entries beginning at 128. Each is four
        // bytes with the block size as a base-two logarithm in its third.
        let mut head = [0u8; 32];
        if !buffer.read_at(0, &mut head) {
            return Err(BringUpError::NoMemory);
        }
        self.blocks = u64::from_le_bytes([
            head[0], head[1], head[2], head[3], head[4], head[5], head[6], head[7],
        ]);

        let format = usize::from(head[26] & 0x0F);
        let mut lbaf = [0u8; 4];
        if !buffer.read_at(128 + format * 4, &mut lbaf) {
            return Err(BringUpError::NoMemory);
        }
        self.block_bytes = 1u32 << lbaf[2];
        Ok(())
    }

    /// Build the one I/O queue pair, with its interrupts disabled.
    fn create_io_queues(&mut self, capacity: Capacity) -> Result<(), BringUpError> {
        let pair = QueuePair::new(IO_QUEUE, capacity).ok_or(BringUpError::NoMemory)?;

        // The completion queue first: a submission queue names the completion
        // queue that answers it, so the answer has to exist.
        //
        // `CDW11` bit 0 is `PC`, physically contiguous, which these are. Bit 1
        // is `IEN`, and it is **left clear**: that is what makes this polled
        // and what removes interrupt sharing from the critical path (`012` §2).
        let completion = Command::new(nvme::admin::CREATE_COMPLETION_QUEUE, self.take_id())
            .buffers(pair.completion.address(), 0)
            .dword(10, (capacity.as_zero_based() << 16) | u32::from(IO_QUEUE))
            .dword(11, 1);
        self.run_admin(completion)?;

        // `CDW11` here is the completion queue's identifier in the high half,
        // and `PC` in bit 0.
        let submission = Command::new(nvme::admin::CREATE_SUBMISSION_QUEUE, self.take_id())
            .buffers(pair.submission.address(), 0)
            .dword(10, (capacity.as_zero_based() << 16) | u32::from(IO_QUEUE))
            .dword(11, (u32::from(IO_QUEUE) << 16) | 1);
        self.run_admin(submission)?;

        self.io = Some(pair);
        Ok(())
    }

    /// Read `blocks` logical blocks starting at `lba` into `into`.
    ///
    /// # Errors
    ///
    /// [`BringUpError::Command`] with the controller's status, or
    /// [`BringUpError::NotReady`] if it never answered.
    pub(crate) fn read_blocks(
        &mut self,
        lba: u64,
        blocks: u32,
        into: &Segment,
    ) -> Result<(), BringUpError> {
        let length = u64::from(blocks) * u64::from(self.block_bytes);
        let built =
            nvme::regions(into.address(), length, PAGE_SIZE, None).ok_or(BringUpError::NoMemory)?;

        let command = Command::new(nvme::nvm::READ, self.take_id())
            .namespace(NAMESPACE)
            .buffers(built.prp1, built.prp2)
            .dword(10, lba as u32)
            .dword(11, (lba >> 32) as u32)
            // **Zero-based**, like every count in this specification: one block
            // is a zero here, and an off-by-one reads a block too many.
            .dword(12, blocks - 1);

        let queue = self.io.is_some();
        if !queue {
            return Err(BringUpError::NotReady);
        }
        self.run_on(IO_QUEUE, command)
    }

    /// The next command identifier.
    fn take_id(&mut self) -> u16 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Run one command on the admin queue.
    fn run_admin(&mut self, command: Command) -> Result<(), BringUpError> {
        self.run_on(0, command)
    }

    /// Submit `command` to queue `which` and spin until it is answered.
    fn run_on(&mut self, which: u16, command: Command) -> Result<(), BringUpError> {
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
        write(&mut self.registers, submission_doorbell, submission_tail)?;

        // Poll for a completion carrying the phase this pass expects. There is
        // no tail to read; the phase tag is the whole of how a new entry is
        // recognised (`012` §3).
        for _ in 0..POLL_ATTEMPTS {
            let found = {
                let queue = self.queue_mut(which).ok_or(BringUpError::NotReady)?;
                let at = queue.head as usize * nvme::COMPLETION_ENTRY_BYTES;
                let mut bytes = [0u8; nvme::COMPLETION_ENTRY_BYTES];
                if !queue.completion.read_at(at, &mut bytes) {
                    return Err(BringUpError::NoMemory);
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
                let doorbell =
                    nvme::doorbell(capabilities, which, true).ok_or(BringUpError::NotReady)?;
                let head = self.queue_mut(which).ok_or(BringUpError::NotReady)?.head;
                write(&mut self.registers, doorbell, head)?;

                if entry.id != expected {
                    return Err(BringUpError::Command(entry.status));
                }
                if !entry.is_success() {
                    return Err(BringUpError::Command(entry.status));
                }
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(BringUpError::NotReady)
    }

    /// The queue pair with this identifier.
    fn queue_mut(&mut self, which: u16) -> Option<&mut QueuePair> {
        if which == self.admin.id {
            return Some(&mut self.admin);
        }
        self.io.as_mut().filter(|queue| queue.id == which)
    }
}

/// Spin until `CSTS.RDY` matches `wanted`, bounded by `CAP.TO`.
///
/// **`CSTS.CFS` ends the wait too.** A controller that has given up says so
/// there, and a driver that only watches the ready bit waits out the whole
/// timeout for an answer that has already arrived.
fn wait_for_ready(
    registers: &mut MmioRegion,
    capabilities: Capabilities,
    wanted: bool,
) -> Result<(), BringUpError> {
    // The timeout is in half-seconds and this loop has no clock, so it is
    // turned into an attempt count generously: the point of reading `CAP.TO`
    // is that a slow controller is not called broken, and the bound is only
    // there so a dead one is.
    let attempts = (capabilities.timeout_ms().max(500) * 20_000).min(20_000_000);
    for _ in 0..attempts {
        let bits = read(registers, register::CSTS)?;
        if bits & status::FATAL != 0 {
            return Err(BringUpError::NotReady);
        }
        if ((bits & status::READY) != 0) == wanted {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(BringUpError::NotReady)
}

fn read(registers: &MmioRegion, offset: usize) -> Result<u32, BringUpError> {
    registers
        .read_u32(offset)
        .map_err(|_| BringUpError::NotMapped)
}

fn write(registers: &mut MmioRegion, offset: usize, value: u32) -> Result<(), BringUpError> {
    registers
        .write_u32(offset, value)
        .map_err(|_| BringUpError::NotMapped)
}

/// Write a sixty-four-bit register as two halves, low first.
///
/// The specification allows it for `ASQ` and `ACQ`, and an `MmioRegion` makes
/// thirty-two-bit accesses. Low first because the controller latches on the
/// high half.
fn write_u64(registers: &mut MmioRegion, offset: usize, value: u64) -> Result<(), BringUpError> {
    write(registers, offset, value as u32)?;
    write(registers, offset + 4, (value >> 32) as u32)
}

/// Bring the controller up and say what it is.
///
/// Declines rather than fails: a machine whose controller will not initialise
/// still boots, and everything built before this works without storage.
pub(crate) fn start(context: &mut ThreadContext, base: u64, len: u64) -> Option<Controller> {
    match bring_up(context, base, len) {
        Ok(controller) => {
            info!(
                "nvme: up — {} entries a queue, {} blocks of {} bytes, doorbells {} apart",
                controller.queue_entries(),
                controller.blocks,
                controller.block_bytes,
                controller.capabilities().doorbell_bytes()
            );
            Some(controller)
        }
        Err(reason) => {
            warn!("nvme: not brought up: {reason}");
            None
        }
    }
}

/// The one controller, kept so anything that wants blocks can ask.
///
/// A `static` rather than a value threaded through boot, because the driver
/// outlives the function that built it and slice 6's reader is not written
/// yet. It becomes a block device behind the VFS in slice 7; this is what it
/// is until then.
static CONTROLLER: SpinLock<Option<(Controller, Segment)>> = SpinLock::new(None);

/// Keep a controller and the buffer it reads into.
pub(crate) fn keep(controller: Controller, buffer: Segment) {
    *CONTROLLER.lock_leaf() = Some((controller, buffer));
}

/// Run `f` against the controller, if there is one.
///
/// A leaf guard, and everything reachable from `f` must respect that: the
/// driver's own path touches memory-mapped registers and direct-mapped frames
/// and takes no lock, which is what makes this sound (D-85).
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
pub(crate) fn with_controller<T>(f: impl FnOnce(&mut Controller, &mut Segment) -> T) -> Option<T> {
    let mut held = CONTROLLER.lock_leaf();
    let (controller, buffer) = held.as_mut()?;
    Some(f(controller, buffer))
}
