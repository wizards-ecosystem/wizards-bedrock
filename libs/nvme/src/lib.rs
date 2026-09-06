//! Register layout, queue arithmetic and command encoding for NVM Express.
//!
//! Everything about talking to an NVMe controller that is not an instruction.
//! `wost` keeps the memory-mapped accesses; the kernel keeps the queues' pages
//! and the waiting. What is here is arithmetic, where `cargo test`, a doctest
//! and a mutation can all reach it.
//!
//! **The ring parallel is a trap** (`012` §3). This kernel already has a
//! submission/completion ring with a doorbell, and two research passes
//! independently concluded: *share the arithmetic, never the structure*. The
//! invariant does generalise — Base 2.2 §3.3.1.2.1 puts the same obligation on
//! a host that D-79 puts on the kernel, never admit work whose completion has
//! nowhere to go — but four things differ and every one of them is a bug if
//! assumed away:
//!
//! - **The completion tail is not visible to the host.** New entries are found
//!   by their **phase tag**, which flips each time the queue wraps.
//! - **Completions are correlated by command identifier**, not by slot. A
//!   controller may complete out of order, and does.
//! - **Capacity is `size - 1`.** A full queue and an empty one are the same
//!   head and tail, so one entry is always left unused. The symptom of getting
//!   it wrong is a **hang**, not an error.
//! - **The doorbell is write-only.** The tail the controller believes is not
//!   readable, so the host shadows it.
//!
//! # Examples
//!
//! ```
//! use nvme::{Capacity, register};
//!
//! // A controller reporting MQES = 63 leads the driver to admit 64 entries.
//! let cap = Capacity::new(64).expect("a real queue size");
//! assert_eq!(cap.entries(), 64);
//!
//! // It can have 63 commands outstanding, never 64.
//! assert_eq!(cap.as_zero_based(), 63);
//!
//! assert_eq!(register::CAP, 0x00);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

/// Bytes of one submission entry. Fixed by the specification at 64.
pub const SUBMISSION_ENTRY_BYTES: usize = 64;

/// Bytes of one completion entry. Fixed by the specification at 16.
pub const COMPLETION_ENTRY_BYTES: usize = 16;

/// `CC.IOSQES`: the base-two logarithm of [`SUBMISSION_ENTRY_BYTES`].
pub const IOSQES: u32 = 6;

/// `CC.IOCQES`: the base-two logarithm of [`COMPLETION_ENTRY_BYTES`].
pub const IOCQES: u32 = 4;

/// Most logical blocks one command can encode in the zero-based NLB field.
pub const MAX_TRANSFER_BLOCKS: u32 = u16::MAX as u32 + 1;

/// A hardware poll bounded either by monotonic time or by boot-time attempts.
///
/// Re-exported from `timebase`, which is where the rule lives now that the
/// keyboard's human wait needs the same one (D-240).
pub use timebase::PollLimit;

/// Logical-block size the current storage caller can consume.
pub const SUPPORTED_BLOCK_BYTES: u32 = 512;

const NAMESPACE_SIZE_BYTES: usize = 8;
const FORMAT_COUNT_OFFSET: usize = 25;
const FORMATTED_OFFSET: usize = 26;
#[cfg(test)]
const METADATA_CAPABILITIES_OFFSET: usize = 27;
#[cfg(test)]
const PROTECTION_CAPABILITIES_OFFSET: usize = 28;
const PROTECTION_SETTINGS_OFFSET: usize = 29;
const LBA_FORMATS_OFFSET: usize = 128;
const LBA_FORMAT_BYTES: usize = 4;
const MAX_LBA_FORMATS: u8 = 16;
const SUPPORTED_BLOCK_SHIFT: u8 = 9;
const IDENTIFY_TRANSFER_BYTES: u64 = 4096;

/// Identify Namespace prefix through all sixteen negotiated LBA formats.
pub const NAMESPACE_IDENTIFY_BYTES: usize =
    LBA_FORMATS_OFFSET + MAX_LBA_FORMATS as usize * LBA_FORMAT_BYTES;

/// Publication state for the controller's sole outstanding command.
///
/// This type records only events which a hardware-free caller can prove. A
/// matching completion returns to idle, while [`DmaState::close`] permanently
/// closes publication. Whether a stopped device makes memory recyclable is a
/// hardware authority and remains at the register-observation boundary.
#[derive(Debug, PartialEq, Eq)]
pub struct DmaState {
    state: DmaOwnership,
}

/// Proof that a prepared DMA command has become device-owned.
///
/// Only [`DmaState::publish_with`] can mint this proof, after changing the
/// reservation to the in-flight state. An MMIO adapter consumes it while
/// ringing the matching submission doorbell.
#[derive(Debug)]
pub struct DmaPublication<'a> {
    _state: &'a DmaState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DmaOwnership {
    Idle,
    Reserved,
    InFlight,
    Closed,
}

impl DmaState {
    /// No command has made memory device-owned yet.
    pub const fn idle() -> Self {
        Self {
            state: DmaOwnership::Idle,
        }
    }

    /// Reserve the sole command slot before preparing its queue entry.
    ///
    /// False while another reservation or command exists, or after controller
    /// shutdown made the ownership decision terminal.
    pub fn reserve(&mut self) -> bool {
        if self.state != DmaOwnership::Idle {
            return false;
        }
        self.state = DmaOwnership::Reserved;
        true
    }

    /// Cancel a reservation that was never published to the controller.
    ///
    /// False unless a caller had successfully reserved the one in-flight
    /// slot. Once a doorbell has been rung, only completion or quiescence may
    /// return ownership instead.
    pub fn cancel_unpublished(&mut self) -> bool {
        if self.state != DmaOwnership::Reserved {
            return false;
        }
        self.state = DmaOwnership::Idle;
        true
    }

    /// Make a prepared command device-owned before invoking its handoff.
    ///
    /// The callback is the only publication window: it receives an opaque
    /// proof after ownership changed, so an MMIO adapter can require that
    /// proof instead of relying on a separately maintained source-order
    /// convention.
    pub fn publish_with<T>(&mut self, handoff: impl FnOnce(DmaPublication<'_>) -> T) -> Option<T> {
        if self.state != DmaOwnership::Reserved {
            return None;
        }
        self.state = DmaOwnership::InFlight;
        Some(handoff(DmaPublication { _state: self }))
    }

    /// Record the matching completion of a published command.
    #[must_use]
    pub fn complete(&mut self) -> bool {
        if self.state != DmaOwnership::InFlight {
            return false;
        }
        self.state = DmaOwnership::Idle;
        true
    }

    /// Permanently refuse another publication on this controller.
    pub fn close(&mut self) {
        self.state = DmaOwnership::Closed;
    }

    /// Whether a submitted command still lacks a matching completion.
    pub const fn is_in_flight(&self) -> bool {
        matches!(self.state, DmaOwnership::InFlight)
    }
}

/// Byte offsets of the controller registers, from Base 2.2 §3.1.
pub mod register {
    /// Controller capabilities. 64 bits, and **read, never assumed** — `012`
    /// §11 records that VMware's values are unmeasured.
    pub const CAP: usize = 0x00;
    /// Controller configuration.
    pub const CC: usize = 0x14;
    /// Controller status.
    pub const CSTS: usize = 0x1C;
    /// Admin queue attributes.
    pub const AQA: usize = 0x24;
    /// Admin submission queue base address.
    pub const ASQ: usize = 0x28;
    /// Admin completion queue base address.
    pub const ACQ: usize = 0x30;
    /// Where the doorbell registers begin.
    pub const DOORBELLS: usize = 0x1000;
}

/// `CC` fields.
pub mod configuration {
    /// Enable.
    pub const ENABLE: u32 = 1;
    /// Shift of `MPS`, the host page size as `log2(size) - 12`.
    pub const PAGE_SIZE_SHIFT: u32 = 7;
    /// Shift of `IOSQES`.
    pub const SUBMISSION_ENTRY_SHIFT: u32 = 16;
    /// Shift of `IOCQES`.
    pub const COMPLETION_ENTRY_SHIFT: u32 = 20;
}

/// `CSTS` fields.
pub mod status {
    /// Ready.
    pub const READY: u32 = 1;
    /// Controller fatal status. A controller that has given up says so here,
    /// and a driver that only waits for `READY` waits for ever.
    pub const FATAL: u32 = 1 << 1;
}

/// Register values that describe a controller unable to execute DMA.
///
/// This value classifies bits; it does not prove where they were observed.
/// Hardware authority must also retain the exact register mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisabledRegisters {
    cc: u32,
    csts: u32,
}

impl DisabledRegisters {
    /// Classify freshly read `CC` and `CSTS` values.
    ///
    /// # Errors
    ///
    /// Returns [`ControllerStateError::Active`] when either `CC.EN` or
    /// `CSTS.RDY` remains set.
    pub const fn from_bits(cc: u32, csts: u32) -> Result<Self, ControllerStateError> {
        if cc & configuration::ENABLE != 0 || csts & status::READY != 0 {
            return Err(ControllerStateError::Active { cc, csts });
        }
        Ok(Self { cc, csts })
    }

    /// The exact `CC` and `CSTS` values that passed classification.
    pub const fn bits(self) -> (u32, u32) {
        (self.cc, self.csts)
    }
}

/// Why controller register values cannot authorize DMA admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ControllerStateError {
    /// The controller remains enabled or ready.
    Active {
        /// Observed `CC` value.
        cc: u32,
        /// Observed `CSTS` value.
        csts: u32,
    },
}

impl core::fmt::Display for ControllerStateError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Active { cc, csts } => {
                write!(
                    formatter,
                    "controller is active (cc={cc:#010x}, csts={csts:#010x})"
                )
            }
        }
    }
}

impl core::error::Error for ControllerStateError {}

/// Exclusive register ownership while controller quiescence is unverified.
///
/// Each fallible access consumes and returns this value, so an error cannot
/// discard the authority needed to retry or quarantine the controller.
#[derive(Debug)]
#[must_use = "unverified controller ownership must be advanced or retained"]
pub struct ControllerPreparation<T> {
    owner: T,
}

/// Register ownership paired with a fresh disabled-controller observation.
#[derive(Debug)]
#[must_use = "disabled controller ownership must be retired or advanced"]
pub struct DisabledPreparation<T> {
    owner: T,
    _observed: DisabledRegisters,
}

/// A refused preparation step with the unchanged ownership state.
#[derive(Debug)]
#[must_use = "a refused preparation step returns its cleanup authority"]
pub struct PreparationFailure<P, E> {
    preparation: P,
    reason: E,
}

impl<T> ControllerPreparation<T> {
    /// Begin preparation with one exclusive register owner.
    pub const fn new(owner: T) -> Self {
        Self { owner }
    }

    /// Attempt one register operation without losing ownership on refusal.
    pub fn access<U, E>(
        mut self,
        operation: impl FnOnce(&mut T) -> Result<U, E>,
    ) -> Result<(Self, U), PreparationFailure<Self, E>> {
        match operation(&mut self.owner) {
            Ok(value) => Ok((self, value)),
            Err(reason) => Err(PreparationFailure {
                preparation: self,
                reason,
            }),
        }
    }

    /// Bind the owner to freshly classified disabled register values.
    pub fn prove_disabled(
        self,
        cc: u32,
        csts: u32,
    ) -> Result<DisabledPreparation<T>, PreparationFailure<Self, ControllerStateError>> {
        match DisabledRegisters::from_bits(cc, csts) {
            Ok(observed) => Ok(DisabledPreparation {
                owner: self.owner,
                _observed: observed,
            }),
            Err(reason) => Err(PreparationFailure {
                preparation: self,
                reason,
            }),
        }
    }
}

impl<T> DisabledPreparation<T> {
    /// Attempt one disabled-controller access with exact retirement fallback.
    pub fn access<U, E>(
        mut self,
        operation: impl FnOnce(&mut T) -> Result<U, E>,
    ) -> Result<(Self, U), PreparationFailure<Self, E>> {
        match operation(&mut self.owner) {
            Ok(value) => Ok((self, value)),
            Err(reason) => Err(PreparationFailure {
                preparation: self,
                reason,
            }),
        }
    }

    /// Recover the register owner after disabled state has been proven.
    pub fn into_owner(self) -> T {
        self.owner
    }
}

impl<P, E> PreparationFailure<P, E> {
    /// Recover the refusal and unchanged preparation authority.
    pub fn into_parts(self) -> (E, P) {
        (self.reason, self.preparation)
    }
}

/// What `CAP` says, decoded.
///
/// **Read, never assumed.** Every field here changes what a correct driver
/// does, and `012` §11 records that VMware's values have not been measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    bits: u64,
}

impl Capabilities {
    /// Decode the `CAP` register.
    pub const fn from_bits(bits: u64) -> Self {
        Self { bits }
    }

    /// `MQES`: the maximum queue size, **minus one**.
    ///
    /// The register holds one less than the number of entries, which is the
    /// specification's own off-by-one and the first place to get it wrong.
    pub const fn max_queue_entries(self) -> u32 {
        (self.bits as u32 & 0xFFFF) + 1
    }

    /// `TO`: the timeout, in units of 500 milliseconds.
    ///
    /// How long the controller may take to become ready. A driver that waits
    /// a fixed time instead is a driver that works on one machine.
    pub const fn timeout_ms(self) -> u64 {
        ((self.bits >> 24) & 0xFF) * 500
    }

    /// `DSTRD`: the doorbell stride, as `log2(bytes) - 2`.
    const fn doorbell_stride(self) -> u32 {
        ((self.bits >> 32) & 0xF) as u32
    }

    /// Bytes between one doorbell and the next.
    ///
    /// `4 << DSTRD`. A driver that assumes four bytes works on every controller
    /// that reports zero and silently writes the wrong register on any that
    /// does not.
    pub const fn doorbell_bytes(self) -> usize {
        4usize << self.doorbell_stride()
    }

    /// `MPSMIN`: the smallest page size the controller supports, as
    /// `log2(size) - 12`.
    const fn min_page_shift(self) -> u32 {
        (((self.bits >> 48) & 0xF) as u32) + 12
    }

    /// `MPSMAX`: the largest, likewise.
    const fn max_page_shift(self) -> u32 {
        (((self.bits >> 52) & 0xF) as u32) + 12
    }

    /// Whether the controller can work in pages of `bytes`.
    pub const fn supports_page_size(self, bytes: u64) -> bool {
        let shift = bytes.trailing_zeros();
        bytes.is_power_of_two() && shift >= self.min_page_shift() && shift <= self.max_page_shift()
    }

    /// Whether the NVM command set is supported.
    ///
    /// `CSS` bit 0. A controller without it is not one this driver can read
    /// from, and finding out at the first command rather than at
    /// initialisation is finding out from a hang.
    pub const fn supports_nvm_command_set(self) -> bool {
        self.bits & (1 << 37) != 0
    }
}

/// How many entries a queue holds, and how many commands may be outstanding.
///
/// The `size - 1` rule, in a type, because it is the off-by-one whose symptom
/// is a hang rather than an error (`012` §3). A queue's head and tail are equal
/// when it is empty, so a producer that filled the last slot could not tell the
/// two apart — one entry is always left unused, on every queue, for ever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    entries: u32,
}

impl Capacity {
    /// The smallest queue the specification allows: two entries, one usable.
    pub const MIN_ENTRIES: u32 = 2;

    /// The largest a queue may be.
    pub const MAX_ENTRIES: u32 = 65536;

    /// Build a capacity from a number of entries.
    ///
    /// # Errors
    ///
    /// `None` below two entries — a queue with one holds nothing at all — or
    /// above 65536, which is what a sixteen-bit index reaches.
    pub const fn new(entries: u32) -> Option<Self> {
        if entries < Self::MIN_ENTRIES || entries > Self::MAX_ENTRIES {
            return None;
        }
        Some(Self { entries })
    }

    /// Entries in the queue.
    pub const fn entries(self) -> u32 {
        self.entries
    }

    /// What goes in `AQA`, and in the create-queue command: one less.
    pub const fn as_zero_based(self) -> u32 {
        self.entries - 1
    }

    /// Bytes a submission queue of this size occupies.
    pub const fn submission_bytes(self) -> u64 {
        self.entries as u64 * SUBMISSION_ENTRY_BYTES as u64
    }

    /// Bytes a completion queue of this size occupies.
    pub const fn completion_bytes(self) -> u64 {
        self.entries as u64 * COMPLETION_ENTRY_BYTES as u64
    }

    /// The index after `index`, wrapping.
    pub const fn next(self, index: u32) -> Option<u32> {
        if index >= self.entries {
            return None;
        }
        if index + 1 == self.entries {
            Some(0)
        } else {
            Some(index + 1)
        }
    }

    /// Whether a queue with this head and tail can take another entry.
    ///
    /// The `size - 1` rule applied: the tail may advance to one short of the
    /// head and no further, because equal means empty.
    pub const fn has_room(self, head: u32, tail: u32) -> Option<bool> {
        if head >= self.entries {
            return None;
        }
        match Self::next(self, tail) {
            Some(next) => Some(next != head),
            None => None,
        }
    }
}

/// Which half of a queue's doorbell pair an operation addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doorbell {
    /// Publish a new submission tail.
    Submission,
    /// Publish a consumed completion head.
    Completion,
}

/// Where a queue's doorbell lives, relative to the register file.
///
/// The formula from Base 2.2 §3.1.24: submission and completion doorbells
/// alternate, and the stride comes from `CAP.DSTRD` rather than being four.
///
/// # Errors
///
/// `None` if the offset overflows, which a plausible stride and queue
/// identifier cannot produce and a hostile pair can.
///
/// # Examples
///
/// ```
/// use nvme::{Capabilities, Doorbell, doorbell};
///
/// // A controller with the usual stride of zero: four bytes apart.
/// let cap = Capabilities::from_bits(0);
/// assert_eq!(doorbell(cap, 0, Doorbell::Submission), Some(0x1000));
/// assert_eq!(doorbell(cap, 0, Doorbell::Completion), Some(0x1004));
/// assert_eq!(doorbell(cap, 1, Doorbell::Submission), Some(0x1008));
/// ```
pub const fn doorbell(cap: Capabilities, queue: u16, doorbell: Doorbell) -> Option<usize> {
    let pair_offset = match doorbell {
        Doorbell::Submission => 0,
        Doorbell::Completion => 1,
    };
    let index = (queue as usize) * 2 + pair_offset;
    match index.checked_mul(cap.doorbell_bytes()) {
        Some(offset) => offset.checked_add(register::DOORBELLS),
        None => None,
    }
}

mod admin {
    pub(super) const CREATE_SUBMISSION_QUEUE: u8 = 0x01;
    pub(super) const CREATE_COMPLETION_QUEUE: u8 = 0x05;
    pub(super) const IDENTIFY: u8 = 0x06;
    pub(super) const SET_FEATURES: u8 = 0x09;
}

mod feature {
    pub(super) const NUMBER_OF_QUEUES: u32 = 0x07;
}

mod nvm {
    pub(super) const READ: u8 = 0x02;
}

/// One validated, sixty-four-byte submission entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    words: [u32; SUBMISSION_ENTRY_BYTES / 4],
}

impl Command {
    const fn empty(opcode: u8, id: u16) -> Self {
        let mut words = [0u32; SUBMISSION_ENTRY_BYTES / 4];
        words[0] = opcode as u32 | ((id as u32) << 16);
        Self { words }
    }

    /// Identify one namespace into one controller page.
    pub const fn identify_namespace(
        id: u16,
        namespace: u32,
        address: u64,
        page_size: u64,
    ) -> Option<Self> {
        if namespace == 0 || !Self::contiguous_extent(address, IDENTIFY_TRANSFER_BYTES, page_size) {
            return None;
        }
        let mut command = Self::empty(admin::IDENTIFY, id)
            .with_namespace(namespace)
            .with_buffers(address, 0);
        command.words[10] = 0;
        Some(command)
    }

    /// Request exact nonzero I/O submission and completion queue counts.
    pub const fn number_of_queues(id: u16, submission: u16, completion: u16) -> Option<Self> {
        if submission == 0 || completion == 0 {
            return None;
        }
        let mut command = Self::empty(admin::SET_FEATURES, id);
        command.words[10] = feature::NUMBER_OF_QUEUES;
        command.words[11] = ((completion as u32 - 1) << 16) | (submission as u32 - 1);
        Some(command)
    }

    const fn with_namespace(mut self, nsid: u32) -> Self {
        self.words[1] = nsid;
        self
    }

    const fn with_buffers(mut self, prp1: u64, prp2: u64) -> Self {
        self.words[6] = prp1 as u32;
        self.words[7] = (prp1 >> 32) as u32;
        self.words[8] = prp2 as u32;
        self.words[9] = (prp2 >> 32) as u32;
        self
    }

    const fn contiguous_extent(address: u64, len: u64, page_size: u64) -> bool {
        page_size >= 4096
            && page_size.is_power_of_two()
            && address.is_multiple_of(page_size)
            && len != 0
            && address.checked_add(len).is_some()
    }

    /// Create one physically contiguous polling completion queue.
    pub const fn create_completion_queue(
        id: u16,
        queue: u16,
        capacity: Capacity,
        address: u64,
        page_size: u64,
    ) -> Option<Self> {
        if queue == 0 || !Self::contiguous_extent(address, capacity.completion_bytes(), page_size) {
            return None;
        }
        let mut command = Self::empty(admin::CREATE_COMPLETION_QUEUE, id).with_buffers(address, 0);
        command.words[10] = (capacity.as_zero_based() << 16) | queue as u32;
        command.words[11] = 1;
        Some(command)
    }

    /// Create one physically contiguous submission queue bound to its completion queue.
    pub const fn create_submission_queue(
        id: u16,
        queue: u16,
        completion_queue: u16,
        capacity: Capacity,
        address: u64,
        page_size: u64,
    ) -> Option<Self> {
        if queue == 0
            || completion_queue == 0
            || !Self::contiguous_extent(address, capacity.submission_bytes(), page_size)
        {
            return None;
        }
        let mut command = Self::empty(admin::CREATE_SUBMISSION_QUEUE, id).with_buffers(address, 0);
        command.words[10] = (capacity.as_zero_based() << 16) | queue as u32;
        command.words[11] = ((completion_queue as u32) << 16) | 1;
        Some(command)
    }

    /// Read a validated namespace extent into one aligned controller page.
    pub const fn read(
        id: u16,
        namespace: u32,
        address: u64,
        page_size: u64,
        transfer: ReadTransfer,
    ) -> Option<Self> {
        if namespace == 0
            || transfer.bytes() > page_size
            || !Self::contiguous_extent(address, transfer.bytes(), page_size)
        {
            return None;
        }
        let lba = transfer.lba();
        let mut command = Self::empty(nvm::READ, id)
            .with_namespace(namespace)
            .with_buffers(address, 0);
        command.words[10] = lba as u32;
        command.words[11] = (lba >> 32) as u32;
        command.words[12] = transfer.zero_based_blocks() as u32;
        Some(command)
    }

    /// The command's identifier.
    pub const fn id(self) -> u16 {
        (self.words[0] >> 16) as u16
    }

    /// The sixty-four bytes to write into the queue.
    pub fn to_bytes(self) -> [u8; SUBMISSION_ENTRY_BYTES] {
        let mut bytes = [0u8; SUBMISSION_ENTRY_BYTES];
        for (index, value) in self.words.into_iter().enumerate() {
            let word = value.to_le_bytes();
            bytes[index * 4] = word[0];
            bytes[index * 4 + 1] = word[1];
            bytes[index * 4 + 2] = word[2];
            bytes[index * 4 + 3] = word[3];
        }
        bytes
    }
}

/// One completion entry, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Completion {
    /// Command-specific result.
    pub result: u32,
    /// Where the controller's submission head is now.
    pub submission_head: u16,
    /// Which submission queue it came from.
    pub submission_queue: u16,
    /// Which command it answers.
    pub id: u16,
    /// The phase tag, which is how a new entry is recognised at all.
    pub phase: bool,
    /// The status field, zero for success.
    pub status: u16,
}

impl Completion {
    /// Decode sixteen bytes.
    pub const fn from_bytes(bytes: [u8; COMPLETION_ENTRY_BYTES]) -> Self {
        let result = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let dword2 = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let dword3 = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        Self {
            result,
            submission_head: dword2 as u16,
            submission_queue: (dword2 >> 16) as u16,
            id: dword3 as u16,
            phase: (dword3 >> 16) & 1 != 0,
            status: (dword3 >> 17) as u16,
        }
    }

    /// Whether this completion proves the expected command was consumed.
    ///
    /// This driver publishes only one command at a time, so the controller's
    /// submission head must equal the tail published for that command. Queue
    /// identity and command identity are part of the same provenance check.
    pub const fn matches_submission(
        self,
        queue: u16,
        command: u16,
        published_tail: u32,
        capacity: Capacity,
    ) -> bool {
        self.submission_queue == queue
            && self.id == command
            && (self.submission_head as u32) < capacity.entries()
            && self.submission_head as u32 == published_tail
    }

    /// Whether the command succeeded.
    pub const fn is_success(self) -> bool {
        self.status == 0
    }
}

/// Namespace geometry proved to match the current storage contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Namespace {
    blocks: u64,
}

/// Why Identify Namespace data cannot describe a supported namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NamespaceError {
    /// The namespace contains no addressable logical blocks.
    Empty,
    /// More formats were reported than this host negotiated.
    TooManyFormats(u16),
    /// The formatted index is beyond the reported table.
    FormatOutsideList {
        /// Index named by `FLBAS`.
        selected: u8,
        /// Formats reported by zero-based `NLBAF`.
        formats: u8,
    },
    /// A reserved bit was set in `FLBAS`.
    ReservedFormatBits(u8),
    /// A reserved bit was set in the selected `LBAF` entry.
    ReservedLbaFormatBits(u8),
    /// The selected format carries metadata this driver cannot transfer.
    Metadata(u16),
    /// The namespace enables protection information this driver cannot transfer.
    Protection(u8),
    /// The selected logical-block exponent is not the supported 512 bytes.
    BlockSize(u8),
}

impl core::fmt::Display for NamespaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => core::fmt::Write::write_str(f, "the namespace has no logical blocks"),
            Self::TooManyFormats(formats) => {
                write!(f, "the namespace reports {formats} lba formats")
            }
            Self::FormatOutsideList { selected, formats } => write!(
                f,
                "lba format {selected} is outside the {formats} reported formats"
            ),
            Self::ReservedFormatBits(bits) => {
                write!(f, "formatted lba size has reserved bits {bits:#04x}")
            }
            Self::ReservedLbaFormatBits(bits) => {
                write!(f, "the selected lba format has reserved bits {bits:#04x}")
            }
            Self::Metadata(bytes) => {
                write!(f, "the selected lba format has {bytes} metadata bytes")
            }
            Self::Protection(bits) => {
                write!(f, "the namespace enables protection settings {bits:#04x}")
            }
            Self::BlockSize(shift) => {
                write!(f, "the selected lba data size exponent is {shift}")
            }
        }
    }
}

impl core::error::Error for NamespaceError {}

impl Namespace {
    /// Parse the Identify Namespace prefix this driver negotiated.
    ///
    /// The high `FLBAS` index bits are ignored because this host does not
    /// enable the 64-format extension. Metadata capability fields are likewise
    /// not settings; the selected format and `DPS` decide the transfer shape.
    ///
    /// # Errors
    ///
    /// [`NamespaceError`] names an empty, malformed or unsupported format.
    pub fn parse(identify: &[u8; NAMESPACE_IDENTIFY_BYTES]) -> Result<Self, NamespaceError> {
        let mut size = [0u8; NAMESPACE_SIZE_BYTES];
        size.copy_from_slice(&identify[..NAMESPACE_SIZE_BYTES]);
        let blocks = u64::from_le_bytes(size);
        if blocks == 0 {
            return Err(NamespaceError::Empty);
        }

        let last_format = identify[FORMAT_COUNT_OFFSET];
        if last_format >= MAX_LBA_FORMATS {
            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));
        }

        let formatted = identify[FORMATTED_OFFSET];
        if formatted & 0x80 != 0 {
            return Err(NamespaceError::ReservedFormatBits(formatted));
        }
        let selected = formatted & 0x0f;
        if selected > last_format {
            return Err(NamespaceError::FormatOutsideList {
                selected,
                formats: last_format + 1,
            });
        }

        let offset = LBA_FORMATS_OFFSET + usize::from(selected) * LBA_FORMAT_BYTES;
        let metadata = u16::from_le_bytes([identify[offset], identify[offset + 1]]);
        let block_shift = identify[offset + 2];
        let control = identify[offset + 3];
        if control & 0xfc != 0 {
            return Err(NamespaceError::ReservedLbaFormatBits(control));
        }
        if metadata != 0 {
            return Err(NamespaceError::Metadata(metadata));
        }
        let protection = identify[PROTECTION_SETTINGS_OFFSET];
        if protection != 0 {
            return Err(NamespaceError::Protection(protection));
        }
        if block_shift != SUPPORTED_BLOCK_SHIFT {
            return Err(NamespaceError::BlockSize(block_shift));
        }

        Ok(Self { blocks })
    }

    /// Logical blocks in the namespace.
    pub const fn blocks(self) -> u64 {
        self.blocks
    }

    /// Bytes in each logical block.
    pub const fn block_bytes(self) -> u32 {
        SUPPORTED_BLOCK_BYTES
    }
}

/// A read whose command extent and DMA capacity have both been validated.
///
/// The fields stay private so command construction cannot accidentally return
/// to raw counts after this boundary. D-195 makes this the only value the
/// kernel's read submission path accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadTransfer {
    lba: u64,
    blocks: u32,
    bytes: u64,
}

/// Why a logical-block transfer cannot be encoded or safely received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransferError {
    /// An NVM read must name at least one logical block.
    Empty,
    /// The zero-based sixteen-bit NLB field cannot encode this count.
    TooManyBlocks,
    /// A zero-byte logical block describes no transferable storage.
    BlockSize,
    /// The logical-block extent wrapped or crossed the namespace end.
    OutsideNamespace,
    /// Multiplying the count by the logical-block size overflowed.
    LengthOverflow,
    /// The DMA allocation is shorter than the command's byte extent.
    BufferTooSmall,
    /// The transfer exceeds the controller limit proved during discovery.
    ControllerLimit,
}

impl core::fmt::Display for TransferError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Write::write_str(
            f,
            match self {
                Self::Empty => "the transfer contains no logical blocks",
                Self::TooManyBlocks => "the transfer does not fit the nlb field",
                Self::BlockSize => "the logical-block size is zero",
                Self::OutsideNamespace => "the transfer crosses the namespace boundary",
                Self::LengthOverflow => "the transfer byte length overflows",
                Self::BufferTooSmall => "the dma buffer is shorter than the transfer",
                Self::ControllerLimit => "the transfer exceeds the proved controller limit",
            },
        )
    }
}

impl core::error::Error for TransferError {}

impl ReadTransfer {
    /// Validate one logical-block read against its namespace and DMA owner.
    ///
    /// # Errors
    ///
    /// Names the first invalid boundary in command order: count encoding,
    /// namespace extent, byte extent, then destination capacity.
    pub const fn new(
        lba: u64,
        blocks: u32,
        namespace_blocks: u64,
        block_bytes: u64,
        buffer_bytes: u64,
    ) -> Result<Self, TransferError> {
        if blocks == 0 {
            return Err(TransferError::Empty);
        }
        if blocks > MAX_TRANSFER_BLOCKS {
            return Err(TransferError::TooManyBlocks);
        }
        if block_bytes == 0 {
            return Err(TransferError::BlockSize);
        }
        let Some(end) = lba.checked_add(blocks as u64) else {
            return Err(TransferError::OutsideNamespace);
        };
        if end > namespace_blocks {
            return Err(TransferError::OutsideNamespace);
        }
        let Some(bytes) = (blocks as u64).checked_mul(block_bytes) else {
            return Err(TransferError::LengthOverflow);
        };
        if bytes > buffer_bytes {
            return Err(TransferError::BufferTooSmall);
        }
        Ok(Self { lba, blocks, bytes })
    }

    /// Bind this request to a byte limit proved for the controller.
    pub const fn within_limit(self, limit: u64) -> Result<Self, TransferError> {
        if self.bytes > limit {
            return Err(TransferError::ControllerLimit);
        }
        Ok(self)
    }

    /// First logical block in the transfer.
    pub const fn lba(self) -> u64 {
        self.lba
    }

    /// Exact byte extent the controller may access.
    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    /// Zero-based NLB encoding for command dword 12.
    pub const fn zero_based_blocks(self) -> u16 {
        (self.blocks - 1) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_disabled_and_not_ready_registers_pass_dma_classification() {
        const OTHER_CC: u32 = 0xa5a5_5a00;
        const OTHER_CSTS: u32 = 0x5a5a_a5a0;

        for enabled in [false, true] {
            for ready in [false, true] {
                let cc = OTHER_CC | (u32::from(enabled) * configuration::ENABLE);
                let csts = OTHER_CSTS | (u32::from(ready) * status::READY);
                let classified = DisabledRegisters::from_bits(cc, csts);
                if enabled || ready {
                    assert_eq!(classified, Err(ControllerStateError::Active { cc, csts }));
                } else {
                    assert_eq!(classified.map(DisabledRegisters::bits), Ok((cc, csts)));
                }
            }
        }
        assert_eq!(
            ControllerStateError::Active { cc: 1, csts: 0 }.to_string(),
            "controller is active (cc=0x00000001, csts=0x00000000)"
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    struct PreparationOwner {
        identity: u64,
        accesses: usize,
    }

    fn refuse_unverified_access(
        identity: u64,
        refused: usize,
        accesses: usize,
    ) -> Result<
        ControllerPreparation<PreparationOwner>,
        PreparationFailure<ControllerPreparation<PreparationOwner>, usize>,
    > {
        let mut preparation = ControllerPreparation::new(PreparationOwner {
            identity,
            accesses: 0,
        });
        for access in 0..accesses {
            let (returned, ()) = preparation.access(|owner| {
                owner.accesses += 1;
                if access == refused {
                    Err(access)
                } else {
                    Ok(())
                }
            })?;
            preparation = returned;
        }
        Ok(preparation)
    }

    #[test]
    fn every_unverified_preparation_access_failure_preserves_its_quarantine_owner() {
        const ACCESSES: usize = 6;

        for refused in 0..ACCESSES {
            let identity = 0xcafe_0000 + refused as u64;
            let failure = refuse_unverified_access(identity, refused, ACCESSES)
                .expect_err("the selected preparation access must refuse");
            let (reason, preparation) = failure.into_parts();
            let (preparation, recovered) = preparation
                .access(|owner| Ok::<_, core::convert::Infallible>(owner.identity))
                .expect("the returned owner remains usable");
            let owner = preparation
                .prove_disabled(0, 0)
                .expect("the recovered owner accepts a valid disabled proof")
                .into_owner();

            assert_eq!(reason, refused);
            assert_eq!(recovered, identity);
            assert_eq!(owner.identity, identity);
            assert_eq!(owner.accesses, refused + 1);
        }

        let identity = 0xcafe_ffff;
        let failure = ControllerPreparation::new(PreparationOwner {
            identity,
            accesses: 0,
        })
        .prove_disabled(configuration::ENABLE, 0)
        .expect_err("an active controller cannot release its preparation owner");
        let (reason, preparation) = failure.into_parts();
        let (_, recovered) = preparation
            .access(|owner| Ok::<_, core::convert::Infallible>(owner.identity))
            .expect("a refused disabled proof returns the owner for quarantine");
        assert_eq!(
            reason,
            ControllerStateError::Active {
                cc: configuration::ENABLE,
                csts: 0,
            }
        );
        assert_eq!(recovered, identity);
    }

    #[test]
    fn every_disabled_preparation_access_failure_returns_the_exact_retirement_owner() {
        const ACCESSES: usize = 5;

        for refused in 0..ACCESSES {
            let identity = 0xd15a_0000 + refused as u64;
            let mut preparation = ControllerPreparation::new(PreparationOwner {
                identity,
                accesses: 0,
            })
            .prove_disabled(0, 0)
            .expect("zeroed controller state is disabled");
            let mut failure = None;
            for access in 0..ACCESSES {
                match preparation.access(|owner| {
                    owner.accesses += 1;
                    if access == refused {
                        Err(access)
                    } else {
                        Ok(())
                    }
                }) {
                    Ok((returned, ())) => preparation = returned,
                    Err(refused) => {
                        failure = Some(refused);
                        break;
                    }
                }
            }
            let failure = failure.expect("the selected disabled access must refuse");
            let (reason, preparation) = failure.into_parts();
            let owner = preparation.into_owner();

            assert_eq!(reason, refused);
            assert_eq!(owner.identity, identity);
            assert_eq!(owner.accesses, refused + 1);
        }
    }

    fn identify(blocks: u64) -> [u8; NAMESPACE_IDENTIFY_BYTES] {
        let mut bytes = [0u8; NAMESPACE_IDENTIFY_BYTES];
        bytes[..8].copy_from_slice(&blocks.to_le_bytes());
        bytes[FORMAT_COUNT_OFFSET] = MAX_LBA_FORMATS - 1;
        for format in 0..MAX_LBA_FORMATS {
            let offset = LBA_FORMATS_OFFSET + usize::from(format) * LBA_FORMAT_BYTES;
            bytes[offset + 2] = SUPPORTED_BLOCK_SHIFT;
        }
        bytes
    }

    #[test]
    fn fixed_wire_sizes_and_bit_positions_match_the_nvme_format() {
        assert_eq!(SUBMISSION_ENTRY_BYTES, 64);
        assert_eq!(COMPLETION_ENTRY_BYTES, 16);
        assert_eq!(IOSQES, 6);
        assert_eq!(IOCQES, 4);
        assert_eq!(MAX_TRANSFER_BLOCKS, 65_536);
        assert_eq!(NAMESPACE_IDENTIFY_BYTES, 192);
        assert_eq!(status::READY, 1);
        assert_eq!(status::FATAL, 2);
    }

    #[test]
    fn every_dma_state_reports_one_consistent_ownership_answer() {
        let cases = [
            (DmaOwnership::Idle, false, false),
            (DmaOwnership::Reserved, false, false),
            (DmaOwnership::InFlight, false, true),
            (DmaOwnership::Closed, true, false),
        ];

        for (state, closed, in_flight) in cases {
            let state = DmaState { state };
            assert_eq!(matches!(state.state, DmaOwnership::Closed), closed);
            assert_eq!(state.is_in_flight(), in_flight);
        }
    }

    #[test]
    fn publication_closes_the_unpublished_cancellation_door() {
        let mut state = DmaState::idle();
        assert!(!state.cancel_unpublished());
        assert_eq!(state.publish_with(|_| ()), None);
        assert!(state.reserve());
        assert!(state.cancel_unpublished());
        assert!(state.reserve(), "cancellation restored the idle state");
        assert!(!state.complete());
        assert_eq!(
            state.publish_with(|publication| publication._state.is_in_flight()),
            Some(true)
        );
        assert!(!state.cancel_unpublished());
        assert_eq!(state.publish_with(|_| ()), None);
        assert!(state.complete());
        assert!(state.reserve(), "completion restored the idle state");
    }

    #[test]
    fn closed_dma_state_cannot_publish_or_complete() {
        for mut state in [
            DmaState::idle(),
            DmaState {
                state: DmaOwnership::Reserved,
            },
            DmaState {
                state: DmaOwnership::InFlight,
            },
        ] {
            state.close();
            assert!(matches!(state.state, DmaOwnership::Closed));
            assert!(!state.reserve());
            assert!(!state.cancel_unpublished());
            assert_eq!(state.publish_with(|_| ()), None);
            assert!(!state.complete());
        }
    }

    /// A queue holds one fewer command than it has entries.
    ///
    /// `012` §3 singles this out: a full queue and an empty one have the same
    /// head and tail, so one slot is always unused — and **the symptom of
    /// getting it wrong is a hang**, not an error. There is no controller
    /// response and nothing to log.
    #[test]
    fn a_queue_holds_one_fewer_command_than_it_has_entries() {
        for entries in [2u32, 3, 8, 64, 1024, 65536] {
            let capacity = Capacity::new(entries).expect("a real queue size");
            assert_eq!(capacity.entries(), entries);
            assert_eq!(capacity.as_zero_based(), entries - 1);
        }
    }

    /// The last slot is never taken, whatever the head is.
    ///
    /// Exhaustive over every head and tail of a small queue, because the rule
    /// is about one specific pair out of every pair and a sample misses it.
    #[test]
    fn a_queue_refuses_the_entry_that_would_make_full_look_empty() {
        let capacity = Capacity::new(4).expect("four entries");

        for head in 0..4 {
            for tail in 0..4 {
                let room = capacity.has_room(head, tail).expect("valid indices");
                let would_wrap_onto_the_head = Capacity::next(capacity, tail) == Some(head);
                assert_eq!(room, !would_wrap_onto_the_head, "head {head}, tail {tail}");
            }
        }

        // And the count that follows from it: three, never four.
        let mut tail = 0;
        let mut admitted = 0;
        for _ in 0..capacity.entries() {
            if capacity.has_room(0, tail) != Some(true) {
                break;
            }
            tail = Capacity::next(capacity, tail).expect("valid tail");
            admitted += 1;
        }
        assert_eq!(admitted, capacity.entries() - 1);
        assert_eq!(admitted, 3);
    }

    #[test]
    fn queue_positions_refuse_every_value_outside_the_capacity() {
        let capacity = Capacity::new(4).expect("four entries");
        for index in 0..4 {
            assert_eq!(Capacity::next(capacity, index), Some((index + 1) % 4));
            assert!(capacity.has_room(index, index).is_some());
        }
        for invalid in [4, 5, u32::MAX] {
            assert_eq!(Capacity::next(capacity, invalid), None);
            assert_eq!(capacity.has_room(invalid, 0), None);
            assert_eq!(capacity.has_room(0, invalid), None);
        }
    }

    #[test]
    fn queue_indices_and_storage_bytes_are_exact() {
        let capacity = Capacity::new(4).expect("four entries");
        assert_eq!(capacity.submission_bytes(), 256);
        assert_eq!(capacity.completion_bytes(), 64);
        assert_eq!(
            (0..4)
                .map(|index| Capacity::next(capacity, index))
                .collect::<Vec<_>>(),
            [Some(1), Some(2), Some(3), Some(0)]
        );
    }

    /// Every capability field is read from its own bits.
    ///
    /// Pairwise distinct values so no two fields can be swapped with this
    /// still passing (D-136), and every one of them changes what a correct
    /// driver does.
    #[test]
    fn every_capability_field_is_read_from_its_own_bits() {
        let bits = 0x0000_0000_0000_00FFu64
            | (0x0A << 24)
            | (0x3 << 32)
            | (1 << 37)
            | (1 << 48)
            | (5 << 52);
        let cap = Capabilities::from_bits(bits);

        assert_eq!(cap.max_queue_entries(), 256);
        assert_eq!(cap.timeout_ms(), 5_000);
        assert_eq!(cap.doorbell_stride(), 3);
        assert_eq!(cap.doorbell_bytes(), 32);
        assert!(cap.supports_nvm_command_set());
        assert_eq!(cap.min_page_shift(), 13);
        assert_eq!(cap.max_page_shift(), 17);

        // Refused outside the reported range, or `CC.MPS` would hold a value
        // the controller rejects and the ready bit would never come.
        assert!(!cap.supports_page_size(4096), "below MPSMIN");
        assert!(cap.supports_page_size(8192));
        assert!(cap.supports_page_size(1 << 17));
        assert!(!cap.supports_page_size(1 << 18), "above MPSMAX");
        assert!(!cap.supports_page_size(6144), "not a power of two");
    }

    /// A controller that says nothing supports nothing.
    ///
    /// The zeroed register, which is what a driver reads from an address it
    /// mapped wrongly — and it must not look like a working controller.
    #[test]
    fn a_silent_controller_supports_nothing() {
        let cap = Capabilities::from_bits(0);
        assert!(!cap.supports_nvm_command_set());
        assert_eq!(cap.timeout_ms(), 0);
        assert_eq!(cap.max_queue_entries(), 1);
        assert!(Capacity::new(cap.max_queue_entries()).is_none());
    }

    /// Doorbells alternate, and the stride is the controller's.
    ///
    /// A driver that assumes four bytes works on every controller reporting a
    /// stride of zero and silently writes a *different queue's* doorbell on
    /// any that does not.
    #[test]
    fn doorbells_alternate_and_use_the_controllers_stride() {
        let narrow = Capabilities::from_bits(0);
        assert_eq!(doorbell(narrow, 0, Doorbell::Submission), Some(0x1000));
        assert_eq!(doorbell(narrow, 0, Doorbell::Completion), Some(0x1004));
        assert_eq!(doorbell(narrow, 1, Doorbell::Submission), Some(0x1008));
        assert_eq!(doorbell(narrow, 1, Doorbell::Completion), Some(0x100C));

        // Stride 2: sixteen bytes apart, so the I/O submission doorbell is at
        // 0x1020 and a driver assuming four would write the admin pair.
        let wide = Capabilities::from_bits(2 << 32);
        assert_eq!(wide.doorbell_bytes(), 16);
        assert_eq!(doorbell(wide, 0, Doorbell::Submission), Some(0x1000));
        assert_eq!(doorbell(wide, 1, Doorbell::Submission), Some(0x1020));
        assert_eq!(doorbell(wide, 1, Doorbell::Completion), Some(0x1030));
    }

    /// A command puts every field where the specification says.
    #[test]
    fn a_command_puts_every_field_where_the_specification_says() {
        const LBA: u64 = 0x8877_6655_0000_0064;
        let transfer = ReadTransfer::new(LBA, 8, LBA + 8, 512, 4096).expect("valid transfer");
        let command =
            Command::read(0xABCD, 1, 0x1122_3344_5566_7000, 4096, transfer).expect("valid read");

        assert_eq!(command.id(), 0xABCD);

        let bytes = command.to_bytes();
        assert_eq!(bytes.len(), 64);
        // CDW0: opcode in the low byte, identifier in the high half.
        assert_eq!(&bytes[0..4], &[0x02, 0x00, 0xCD, 0xAB]);
        // NSID.
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
        // PRP1 at 24, PRP2 at 32 — the two most consequential offsets here.
        assert_eq!(&bytes[24..32], &0x1122_3344_5566_7000u64.to_le_bytes());
        assert_eq!(&bytes[32..40], &0u64.to_le_bytes());
        assert_eq!(&bytes[40..48], &LBA.to_le_bytes());
        assert_eq!(&bytes[48..52], &7u32.to_le_bytes());
    }

    #[test]
    fn queue_negotiation_requests_and_proves_both_queue_domains() {
        let command = Command::number_of_queues(0x1234, 1, 1).expect("one queue pair");
        let bytes = command.to_bytes();
        assert_eq!(&bytes[0..4], &[admin::SET_FEATURES, 0, 0x34, 0x12]);
        assert_eq!(&bytes[40..44], &feature::NUMBER_OF_QUEUES.to_le_bytes());
        assert_eq!(&bytes[44..48], &0u32.to_le_bytes());
        assert!(Command::number_of_queues(1, 0, 1).is_none());
        assert!(Command::number_of_queues(1, 1, 0).is_none());
    }

    /// A completion is decoded field by field, and the phase tag is one bit.
    ///
    /// The phase is how a new entry is recognised at all — there is no tail to
    /// read — so a decoder that took it from the wrong bit would see either
    /// every entry or none.
    #[test]
    fn a_completion_is_decoded_field_by_field() {
        let mut bytes = [0u8; COMPLETION_ENTRY_BYTES];
        bytes[0..4].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        // DW2: submission head 0x0011, submission queue 0x0022.
        bytes[8..12].copy_from_slice(&0x0022_0011u32.to_le_bytes());
        // DW3: command id 0x3344, phase set, status 0x0005.
        bytes[12..16].copy_from_slice(&((0x0005u32 << 17) | (1 << 16) | 0x3344).to_le_bytes());

        let completion = Completion::from_bytes(bytes);
        assert_eq!(completion.result, 0x1234_5678);
        assert_eq!(completion.submission_head, 0x0011);
        assert_eq!(completion.submission_queue, 0x0022);
        assert_eq!(completion.id, 0x3344);
        assert!(completion.phase);
        assert_eq!(completion.status, 5);
        assert!(!completion.is_success());

        // The phase bit must not leak into the status, which is the mistake
        // that makes every completion look like a failure.
        bytes[12..16].copy_from_slice(&(1u32 << 16).to_le_bytes());
        let clean = Completion::from_bytes(bytes);
        assert!(clean.phase);
        assert_eq!(clean.status, 0);
        assert!(clean.is_success());

        let empty = Completion::from_bytes([0; COMPLETION_ENTRY_BYTES]);
        assert!(!empty.phase);
        assert_eq!(empty.status, 0);
    }

    #[test]
    fn completion_provenance_binds_queue_command_and_submission_head() {
        let capacity = Capacity::new(8).expect("queue capacity");
        let completion = Completion {
            result: 0,
            submission_head: 3,
            submission_queue: 1,
            id: 0x1234,
            phase: true,
            status: 0,
        };

        assert!(completion.matches_submission(1, 0x1234, 3, capacity));
        assert!(!completion.matches_submission(0, 0x1234, 3, capacity));
        assert!(!completion.matches_submission(1, 0x1235, 3, capacity));
        assert!(!completion.matches_submission(1, 0x1234, 2, capacity));

        let outside = Completion {
            submission_head: 8,
            ..completion
        };
        assert!(!outside.matches_submission(1, 0x1234, 8, capacity));
    }

    #[test]
    fn namespace_errors_name_the_refused_geometry() {
        let cases = [
            (NamespaceError::Empty, "the namespace has no logical blocks"),
            (
                NamespaceError::TooManyFormats(17),
                "the namespace reports 17 lba formats",
            ),
            (
                NamespaceError::FormatOutsideList {
                    selected: 3,
                    formats: 2,
                },
                "lba format 3 is outside the 2 reported formats",
            ),
            (
                NamespaceError::ReservedFormatBits(0x80),
                "formatted lba size has reserved bits 0x80",
            ),
            (
                NamespaceError::ReservedLbaFormatBits(0xfc),
                "the selected lba format has reserved bits 0xfc",
            ),
            (
                NamespaceError::Metadata(8),
                "the selected lba format has 8 metadata bytes",
            ),
            (
                NamespaceError::Protection(3),
                "the namespace enables protection settings 0x03",
            ),
            (
                NamespaceError::BlockSize(12),
                "the selected lba data size exponent is 12",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn transfer_errors_name_the_refused_extent() {
        let cases = [
            (
                TransferError::Empty,
                "the transfer contains no logical blocks",
            ),
            (
                TransferError::TooManyBlocks,
                "the transfer does not fit the nlb field",
            ),
            (TransferError::BlockSize, "the logical-block size is zero"),
            (
                TransferError::OutsideNamespace,
                "the transfer crosses the namespace boundary",
            ),
            (
                TransferError::LengthOverflow,
                "the transfer byte length overflows",
            ),
            (
                TransferError::BufferTooSmall,
                "the dma buffer is shorter than the transfer",
            ),
            (
                TransferError::ControllerLimit,
                "the transfer exceeds the proved controller limit",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    /// A zero-sized namespace has no valid LBA, while both extent ends survive.
    #[test]
    fn a_namespace_has_at_least_one_logical_block() {
        assert_eq!(Namespace::parse(&identify(0)), Err(NamespaceError::Empty));

        let one = Namespace::parse(&identify(1)).expect("one addressable block");
        assert_eq!(one.blocks(), 1);
        assert_eq!(one.block_bytes(), SUPPORTED_BLOCK_BYTES);

        let largest = Namespace::parse(&identify(u64::MAX)).expect("the largest lba extent");
        assert_eq!(largest.blocks(), u64::MAX);
    }

    /// Exhaust the reported count and every byte value of the format selector.
    #[test]
    fn every_format_selector_stays_inside_the_negotiated_table() {
        for last in 0..=u8::MAX {
            let mut bytes = identify(1);
            bytes[FORMAT_COUNT_OFFSET] = last;
            let parsed = Namespace::parse(&bytes);
            if last < MAX_LBA_FORMATS {
                assert!(parsed.is_ok(), "zero-based format count {last}");
            } else {
                assert_eq!(
                    parsed,
                    Err(NamespaceError::TooManyFormats(u16::from(last) + 1)),
                    "zero-based format count {last}"
                );
            }
        }

        for last in 0..MAX_LBA_FORMATS {
            for formatted in 0..=u8::MAX {
                let mut bytes = identify(1);
                bytes[FORMAT_COUNT_OFFSET] = last;
                bytes[FORMATTED_OFFSET] = formatted;
                let selected = formatted & 0x0f;
                let parsed = Namespace::parse(&bytes);
                if formatted & 0x80 != 0 {
                    assert_eq!(
                        parsed,
                        Err(NamespaceError::ReservedFormatBits(formatted)),
                        "last {last}, flbas {formatted:#04x}"
                    );
                } else if selected > last {
                    assert_eq!(
                        parsed,
                        Err(NamespaceError::FormatOutsideList {
                            selected,
                            formats: last + 1,
                        }),
                        "last {last}, flbas {formatted:#04x}"
                    );
                } else {
                    assert!(
                        parsed.is_ok(),
                        "the high index and metadata-placement bits are inapplicable: last {last}, flbas {formatted:#04x}"
                    );
                }
            }
        }
    }

    /// Every possible exponent has one policy, and none is ever shifted first.
    #[test]
    fn every_lbads_value_has_one_explicit_policy() {
        for block_shift in 0..=u8::MAX {
            let mut bytes = identify(1);
            bytes[LBA_FORMATS_OFFSET + 2] = block_shift;
            let parsed = Namespace::parse(&bytes);
            if block_shift == SUPPORTED_BLOCK_SHIFT {
                assert_eq!(
                    parsed.expect("the one supported exponent").block_bytes(),
                    512
                );
            } else {
                assert_eq!(parsed, Err(NamespaceError::BlockSize(block_shift)));
            }
        }
    }

    /// Metadata capabilities are harmless; selected metadata and protection are not.
    #[test]
    fn every_metadata_and_protection_setting_is_explicit() {
        for metadata in 0..=u16::MAX {
            let mut bytes = identify(1);
            bytes[LBA_FORMATS_OFFSET..LBA_FORMATS_OFFSET + 2]
                .copy_from_slice(&metadata.to_le_bytes());
            let parsed = Namespace::parse(&bytes);
            if metadata == 0 {
                assert!(parsed.is_ok());
            } else {
                assert_eq!(parsed, Err(NamespaceError::Metadata(metadata)));
            }
        }

        for protection in 0..=u8::MAX {
            let mut bytes = identify(1);
            bytes[PROTECTION_SETTINGS_OFFSET] = protection;
            let parsed = Namespace::parse(&bytes);
            if protection == 0 {
                assert!(parsed.is_ok());
            } else {
                assert_eq!(parsed, Err(NamespaceError::Protection(protection)));
            }
        }

        for capabilities in 0..=u8::MAX {
            let mut bytes = identify(1);
            bytes[METADATA_CAPABILITIES_OFFSET] = capabilities;
            bytes[PROTECTION_CAPABILITIES_OFFSET] = capabilities;
            assert!(
                Namespace::parse(&bytes).is_ok(),
                "capabilities {capabilities:#04x} do not enable a transfer setting"
            );
        }
    }

    /// Relative performance is descriptive; every higher LBAF bit is reserved.
    #[test]
    fn every_lba_format_control_value_has_one_explicit_policy() {
        for control in 0..=u8::MAX {
            let mut bytes = identify(1);
            bytes[LBA_FORMATS_OFFSET + 3] = control;
            let parsed = Namespace::parse(&bytes);
            if control <= 3 {
                assert!(parsed.is_ok(), "relative performance {control}");
            } else {
                assert_eq!(parsed, Err(NamespaceError::ReservedLbaFormatBits(control)));
            }
        }
    }

    /// Zero has no NLB encoding and must never reach `blocks - 1`.
    #[test]
    fn an_empty_transfer_has_no_nlb_encoding() {
        assert_eq!(
            ReadTransfer::new(0, 0, 1, 512, 512),
            Err(TransferError::Empty)
        );
    }

    /// Exhaust every count around the sixteen-bit, zero-based NLB field.
    #[test]
    fn every_nonempty_nlb_count_has_one_exact_boundary() {
        let namespace = u64::from(MAX_TRANSFER_BLOCKS) + 1;
        for blocks in 1..=MAX_TRANSFER_BLOCKS + 1 {
            let transfer = ReadTransfer::new(0, blocks, namespace, 1, namespace);
            if blocks <= MAX_TRANSFER_BLOCKS {
                let transfer = transfer.expect("the NLB field can encode this count");
                assert_eq!(u32::from(transfer.zero_based_blocks()), blocks - 1);
                assert_eq!(transfer.bytes(), u64::from(blocks));
            } else {
                assert_eq!(transfer, Err(TransferError::TooManyBlocks));
            }
        }
    }

    /// The namespace end is exclusive and its addition never wraps.
    #[test]
    fn a_transfer_never_crosses_or_wraps_its_namespace() {
        let last = ReadTransfer::new(9, 1, 10, 512, 512).expect("the final block");
        assert_eq!(last.lba(), 9);
        assert_eq!(last.bytes(), 512);
        assert_eq!(
            ReadTransfer::new(9, 2, 10, 512, 1024),
            Err(TransferError::OutsideNamespace)
        );
        assert_eq!(
            ReadTransfer::new(u64::MAX, 1, u64::MAX, 1, 1),
            Err(TransferError::OutsideNamespace)
        );
        assert_eq!(
            ReadTransfer::new(0, 1, 0, 1, 1),
            Err(TransferError::OutsideNamespace)
        );
    }

    /// The byte extent must exist, multiply exactly and fit the DMA owner.
    #[test]
    fn transfer_bytes_must_fit_the_dma_buffer() {
        assert_eq!(
            ReadTransfer::new(0, 1, 1, 0, 0),
            Err(TransferError::BlockSize)
        );
        assert_eq!(
            ReadTransfer::new(0, 2, 2, u64::MAX, u64::MAX),
            Err(TransferError::LengthOverflow)
        );
        assert_eq!(
            ReadTransfer::new(0, 2, 2, 512, 1023),
            Err(TransferError::BufferTooSmall)
        );
        let exact = ReadTransfer::new(0, 2, 2, 512, 1024).expect("an exact buffer");
        assert_eq!(exact.bytes(), 1024);
    }

    #[test]
    fn an_unidentified_controller_is_bounded_to_one_page() {
        let exact = ReadTransfer::new(0, 8, 32, 512, 4096).expect("one page");
        assert_eq!(exact.within_limit(4096), Ok(exact));

        let over = ReadTransfer::new(0, 9, 32, 512, 4608).expect("encodable transfer");
        assert_eq!(over.within_limit(4096), Err(TransferError::ControllerLimit));
    }

    #[test]
    fn identify_binds_one_complete_aligned_page() {
        const PAGE: u64 = 4096;
        const LAST_VALID_PAGE: u64 = u64::MAX - (2 * PAGE - 1);
        const WRAPPING_PAGE: u64 = u64::MAX - (PAGE - 1);

        let command = Command::identify_namespace(7, 1, LAST_VALID_PAGE, PAGE)
            .expect("the last non-wrapping page");
        let bytes = command.to_bytes();
        assert_eq!(&bytes[24..32], &LAST_VALID_PAGE.to_le_bytes());
        assert_eq!(&bytes[32..40], &0u64.to_le_bytes());

        assert!(Command::identify_namespace(7, 0, 0x1000, PAGE).is_none());
        assert!(Command::identify_namespace(7, 1, 0x1001, PAGE).is_none());
        assert!(Command::identify_namespace(7, 1, 0x1000, 2048).is_none());
        assert!(Command::identify_namespace(7, 1, 0x2000, 5000).is_none());
        assert!(Command::identify_namespace(7, 1, WRAPPING_PAGE, PAGE).is_none());
    }

    #[test]
    fn read_binds_its_validated_extent_to_one_page() {
        const PAGE: u64 = 4096;
        const LAST_VALID_PAGE: u64 = u64::MAX - (2 * PAGE - 1);
        const WRAPPING_PAGE: u64 = u64::MAX - (PAGE - 1);
        let exact = ReadTransfer::new(0, 8, 32, 512, PAGE).expect("one-page transfer");
        let too_large = ReadTransfer::new(0, 9, 32, 512, PAGE + 512).expect("encodable transfer");

        assert!(Command::read(9, 1, LAST_VALID_PAGE, PAGE, exact).is_some());
        assert!(Command::read(9, 0, 0x1000, PAGE, exact).is_none());
        assert!(Command::read(9, 1, 0x1001, PAGE, exact).is_none());
        assert!(Command::read(9, 1, 0x1000, 2048, exact).is_none());
        assert!(Command::read(9, 1, 0x2000, 5000, exact).is_none());
        assert!(Command::read(9, 1, WRAPPING_PAGE, PAGE, exact).is_none());
        assert!(Command::read(9, 1, 0x1000, PAGE, too_large).is_none());
    }

    #[test]
    fn queue_commands_encode_every_dword_field_where_the_specification_puts_it() {
        const PAGE: u64 = 4096;
        const HIGH: u64 = 0x1_2345_6000;
        let capacity = Capacity::new(256).expect("a queue capacity");

        let completion = Command::create_completion_queue(11, 3, capacity, HIGH, PAGE)
            .expect("a completion queue above 4 GiB");
        let bytes = completion.to_bytes();
        assert_eq!(
            &bytes[24..32],
            &HIGH.to_le_bytes(),
            "PRP1 carries both halves"
        );
        assert_eq!(
            &bytes[40..44],
            &((255u32 << 16) | 3).to_le_bytes(),
            "QSIZE | QID"
        );
        assert_eq!(&bytes[44..48], &1u32.to_le_bytes(), "physically contiguous");

        let submission = Command::create_submission_queue(12, 5, 3, capacity, HIGH, PAGE)
            .expect("a submission queue bound to its completion queue");
        let bytes = submission.to_bytes();
        assert_eq!(&bytes[24..32], &HIGH.to_le_bytes());
        assert_eq!(
            &bytes[40..44],
            &((255u32 << 16) | 5).to_le_bytes(),
            "QSIZE | QID"
        );
        assert_eq!(
            &bytes[44..48],
            &((3u32 << 16) | 1).to_le_bytes(),
            "CQID | PC"
        );

        let queues = Command::number_of_queues(0x1234, 3, 5).expect("three and five queues");
        assert_eq!(
            &queues.to_bytes()[44..48],
            &((4u32 << 16) | 2).to_le_bytes(),
            "NCQR | NSQR, zero based"
        );
    }

    #[test]
    fn queue_commands_bind_the_complete_contiguous_extent() {
        const PAGE: u64 = 4096;
        const WRAPPING_PAGE: u64 = u64::MAX - (PAGE - 1);
        let capacity = Capacity::new(256).expect("a queue capacity");

        let completion = Command::create_completion_queue(11, 1, capacity, 0x1000, PAGE)
            .expect("a complete completion queue");
        let submission = Command::create_submission_queue(12, 1, 1, capacity, 0x2000, PAGE)
            .expect("a complete submission queue");
        assert_eq!(&completion.to_bytes()[24..32], &0x1000u64.to_le_bytes());
        assert_eq!(&submission.to_bytes()[24..32], &0x2000u64.to_le_bytes());

        assert!(Command::create_completion_queue(11, 0, capacity, 0x1000, PAGE).is_none());
        assert!(Command::create_completion_queue(11, 1, capacity, 0x1001, PAGE).is_none());
        assert!(Command::create_completion_queue(11, 1, capacity, WRAPPING_PAGE, PAGE).is_none());
        assert!(Command::create_submission_queue(12, 0, 1, capacity, 0x2000, PAGE).is_none());
        assert!(Command::create_submission_queue(12, 1, 0, capacity, 0x2000, PAGE).is_none());
        assert!(Command::create_submission_queue(12, 1, 1, capacity, 0x2001, PAGE).is_none());
        assert!(
            Command::create_submission_queue(12, 1, 1, capacity, WRAPPING_PAGE, PAGE).is_none()
        );
    }
}
