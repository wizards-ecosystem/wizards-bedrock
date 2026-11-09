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
            .with_buffer(address);
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

    // Every transfer is one contiguous extent, so PRP2 stays the zero `empty` wrote.
    const fn with_buffer(mut self, prp1: u64) -> Self {
        self.words[6] = prp1 as u32;
        self.words[7] = (prp1 >> 32) as u32;
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
        let mut command = Self::empty(admin::CREATE_COMPLETION_QUEUE, id).with_buffer(address);
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
        let mut command = Self::empty(admin::CREATE_SUBMISSION_QUEUE, id).with_buffer(address);
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
            .with_buffer(address);
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
mod tests;
