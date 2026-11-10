//! Register layout, queue arithmetic and command encoding for NVM Express.
//!
//! # Examples
//!
//! ```
//! use nvme::{Capacity, register};
//!
//! // A controller reporting MQES = 63 leads the driver to admit 64 entries.
//! let capacity = Capacity::new(64).expect("a real queue size");
//! assert_eq!(capacity.entries(), 64);
//!
//! // It can have 63 commands outstanding, never 64.
//! assert_eq!(capacity.as_zero_based(), 63);
//!
//! assert_eq!(register::CAP, 0x00);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

pub use timebase::PollLimit;

use core::fmt;

pub const SUBMISSION_ENTRY_BYTES: usize = 64;

pub const COMPLETION_ENTRY_BYTES: usize = 16;

/// `CC.IOSQES`: the base-two logarithm of [`SUBMISSION_ENTRY_BYTES`].
pub const IOSQES: u32 = 6;

/// `CC.IOCQES`: the base-two logarithm of [`COMPLETION_ENTRY_BYTES`].
pub const IOCQES: u32 = 4;

/// Most logical blocks one command can encode in the zero-based NLB field.
pub const MAX_TRANSFER_BLOCKS: u32 = u16::MAX as u32 + 1;

/// Logical-block size the current storage caller can consume.
pub const SUPPORTED_BLOCK_BYTES: u32 = 512;

const NAMESPACE_SIZE_BYTES: usize = 8;
const LBA_FORMAT_BYTES: usize = 4;
const MAX_LBA_FORMATS: u8 = 16;
const SUPPORTED_BLOCK_SHIFT: u8 = 9;
const IDENTIFY_TRANSFER_BYTES: u64 = 4096;

/// Byte offsets of the Identify Namespace data structure fields.
mod identify {
    /// `NLBAF`, the zero-based number of LBA formats.
    pub(super) const FORMAT_COUNT: usize = 25;
    /// `FLBAS`, the formatted LBA size.
    pub(super) const FORMATTED: usize = 26;
    /// `MC`, metadata capabilities.
    #[cfg(test)]
    pub(super) const METADATA_CAPABILITIES: usize = 27;
    /// `DPC`, end-to-end data protection capabilities.
    #[cfg(test)]
    pub(super) const PROTECTION_CAPABILITIES: usize = 28;
    /// `DPS`, end-to-end data protection type settings.
    pub(super) const PROTECTION_SETTINGS: usize = 29;
    /// `LBAF0`, the first LBA format descriptor.
    pub(super) const LBA_FORMATS: usize = 128;
}

/// Identify Namespace prefix through all sixteen negotiated LBA formats.
pub const NAMESPACE_IDENTIFY_BYTES: usize =
    identify::LBA_FORMATS + MAX_LBA_FORMATS as usize * LBA_FORMAT_BYTES;

/// Publication state for the controller's sole outstanding command.
/// [`Self::close`] frees no DMA memory; only a completion or verified quiescence returns it.
#[derive(PartialEq, Eq, Debug)]
pub struct DmaState {
    state: DmaOwnership,
}

/// Proof, minted only by [`DmaState::publish_with`], that a prepared command is device-owned.
/// The MMIO adapter consumes it while ringing the matching submission doorbell.
#[derive(Debug)]
pub struct DmaPublication<'state> {
    _state: &'state DmaState,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DmaOwnership {
    Idle,
    Reserved,
    InFlight,
    Closed,
}

impl DmaState {
    pub const fn idle() -> Self {
        Self {
            state: DmaOwnership::Idle,
        }
    }

    /// Reserve the sole command slot, refused while another command exists or after close.
    pub fn reserve(&mut self) -> bool {
        if self.state != DmaOwnership::Idle {
            return false;
        }
        self.state = DmaOwnership::Reserved;
        true
    }

    /// Cancel a reservation that was never published to the controller.
    pub fn cancel_unpublished(&mut self) -> bool {
        if self.state != DmaOwnership::Reserved {
            return false;
        }
        self.state = DmaOwnership::Idle;
        true
    }

    /// Make a prepared command device-owned, then hand the proof to `handoff`.
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
    /// Controller capabilities, 64 bits.
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
    /// `EN`, bit 0.
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
    /// `RDY`, bit 0.
    pub const READY: u32 = 1;
    /// `CFS`, bit 1: the controller has given up and will never become ready.
    pub const FATAL: u32 = 1 << 1;
}

/// `CC` and `CSTS` values that describe a controller unable to execute DMA.
/// They prove nothing about where they were read, so hardware authority must keep the mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DisabledRegisters {
    cc: u32,
    csts: u32,
}

impl DisabledRegisters {
    /// Classify freshly read `CC` and `CSTS` values, refusing either `CC.EN` or `CSTS.RDY`.
    pub const fn from_bits(cc: u32, csts: u32) -> Result<Self, ControllerStateError> {
        if cc & configuration::ENABLE != 0 || csts & status::READY != 0 {
            return Err(ControllerStateError::Active { cc, csts });
        }
        Ok(Self { cc, csts })
    }

    pub const fn bits(self) -> (u32, u32) {
        (self.cc, self.csts)
    }
}

/// Why controller register values cannot authorize DMA admission.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ControllerStateError {
    /// The controller remains enabled or ready.
    Active { cc: u32, csts: u32 },
}

impl fmt::Display for ControllerStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Active { cc, csts } => {
                write!(f, "controller is active (cc={cc:#010x}, csts={csts:#010x})")
            }
        }
    }
}

impl core::error::Error for ControllerStateError {}

/// Register ownership, returned by every fallible access, while quiescence is unverified.
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

/// What `CAP` says, decoded, because every field changes what a correct driver does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capabilities {
    bits: u64,
}

impl Capabilities {
    pub const fn from_bits(bits: u64) -> Self {
        Self { bits }
    }

    /// `MQES` plus one, since the register holds one less than the number of entries.
    pub const fn max_queue_entries(self) -> u32 {
        (self.bits as u32 & 0xFFFF) + 1
    }

    /// `TO`, the time the controller may take to become ready, from units of 500 milliseconds.
    pub const fn timeout_ms(self) -> u64 {
        ((self.bits >> 24) & 0xFF) * 500
    }

    /// `DSTRD`: the doorbell stride, as `log2(bytes) - 2`.
    const fn doorbell_stride(self) -> u32 {
        ((self.bits >> 32) & 0xF) as u32
    }

    /// Bytes between one doorbell and the next, `4 << DSTRD`.
    pub const fn doorbell_bytes(self) -> usize {
        4usize << self.doorbell_stride()
    }

    /// `MPSMIN`: the smallest page size the controller supports, as `log2(size)`.
    const fn min_page_shift(self) -> u32 {
        (((self.bits >> 48) & 0xF) as u32) + 12
    }

    /// `MPSMAX`: the largest page size, as `log2(size)`.
    const fn max_page_shift(self) -> u32 {
        (((self.bits >> 52) & 0xF) as u32) + 12
    }

    pub const fn supports_page_size(self, bytes: u64) -> bool {
        let shift = bytes.trailing_zeros();
        bytes.is_power_of_two() && shift >= self.min_page_shift() && shift <= self.max_page_shift()
    }

    /// Whether `CAP` bit 37, which is `CSS` bit 0, advertises the NVM command set.
    pub const fn supports_nvm_command_set(self) -> bool {
        self.bits & (1 << 37) != 0
    }
}

/// Queue entries, of which one always stays unused because equal head and tail means empty.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capacity {
    entries: u32,
}

impl Capacity {
    /// The smallest queue the specification allows: two entries, one usable.
    pub const MIN_ENTRIES: u32 = 2;

    pub const MAX_ENTRIES: u32 = 65536;

    /// A capacity of `entries`, refused below two or above what a sixteen-bit index reaches.
    pub const fn new(entries: u32) -> Option<Self> {
        if entries < Self::MIN_ENTRIES || entries > Self::MAX_ENTRIES {
            return None;
        }
        Some(Self { entries })
    }

    pub const fn entries(self) -> u32 {
        self.entries
    }

    /// What goes in `AQA`, and in the create-queue command: one less.
    pub const fn as_zero_based(self) -> u32 {
        self.entries - 1
    }

    pub const fn submission_bytes(self) -> u64 {
        self.entries as u64 * SUBMISSION_ENTRY_BYTES as u64
    }

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

    /// Whether the tail may advance, which stops one short of the head.
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
/// Doorbells are write-only, so the host shadows the tail it last published.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Doorbell {
    /// Publish a new submission tail.
    Submission,
    /// Publish a consumed completion head.
    Completion,
}

/// Where a queue's doorbell lives, by Base 2.2 §3.1.24, or `None` if the offset overflows.
///
/// # Examples
///
/// ```
/// use nvme::{Capabilities, Doorbell, doorbell};
///
/// // A controller with the usual stride of zero: four bytes apart.
/// let capabilities = Capabilities::from_bits(0);
/// assert_eq!(doorbell(capabilities, 0, Doorbell::Submission), Some(0x1000));
/// assert_eq!(doorbell(capabilities, 0, Doorbell::Completion), Some(0x1004));
/// assert_eq!(doorbell(capabilities, 1, Doorbell::Submission), Some(0x1008));
/// ```
pub const fn doorbell(capabilities: Capabilities, queue: u16, doorbell: Doorbell) -> Option<usize> {
    let pair_offset = match doorbell {
        Doorbell::Submission => 0,
        Doorbell::Completion => 1,
    };
    let index = (queue as usize) * 2 + pair_offset;
    match index.checked_mul(capabilities.doorbell_bytes()) {
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Completion {
    /// Command-specific result.
    pub result: u32,
    pub submission_head: u16,
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

    /// Whether queue, command and a head equal to the published tail prove the command consumed.
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

    pub const fn is_success(self) -> bool {
        self.status == 0
    }
}

/// Namespace geometry proved to match the current storage contract.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Namespace {
    blocks: u64,
}

/// Why Identify Namespace data cannot describe a supported namespace.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum NamespaceError {
    /// The namespace contains no addressable logical blocks.
    Empty,
    /// More formats were reported than this host negotiated.
    TooManyFormats(u16),
    /// The `FLBAS` index is beyond the zero-based `NLBAF` table.
    FormatOutsideList { selected: u8, formats: u8 },
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

impl fmt::Display for NamespaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the namespace has no logical blocks"),
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
    /// Parse the negotiated Identify Namespace prefix, ignoring the high `FLBAS` index bits.
    pub fn parse(identify: &[u8; NAMESPACE_IDENTIFY_BYTES]) -> Result<Self, NamespaceError> {
        let mut size = [0u8; NAMESPACE_SIZE_BYTES];
        size.copy_from_slice(&identify[..NAMESPACE_SIZE_BYTES]);
        let blocks = u64::from_le_bytes(size);
        if blocks == 0 {
            return Err(NamespaceError::Empty);
        }

        let last_format = identify[identify::FORMAT_COUNT];
        if last_format >= MAX_LBA_FORMATS {
            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));
        }

        let formatted = identify[identify::FORMATTED];
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

        let offset = identify::LBA_FORMATS + usize::from(selected) * LBA_FORMAT_BYTES;
        let metadata = u16::from_le_bytes([identify[offset], identify[offset + 1]]);
        let block_shift = identify[offset + 2];
        let control = identify[offset + 3];
        if control & 0xfc != 0 {
            return Err(NamespaceError::ReservedLbaFormatBits(control));
        }
        if metadata != 0 {
            return Err(NamespaceError::Metadata(metadata));
        }
        let protection = identify[identify::PROTECTION_SETTINGS];
        if protection != 0 {
            return Err(NamespaceError::Protection(protection));
        }
        if block_shift != SUPPORTED_BLOCK_SHIFT {
            return Err(NamespaceError::BlockSize(block_shift));
        }

        Ok(Self { blocks })
    }

    pub const fn blocks(self) -> u64 {
        self.blocks
    }

    pub const fn block_bytes(self) -> u32 {
        SUPPORTED_BLOCK_BYTES
    }
}

/// A read whose command extent and DMA capacity have both been validated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReadTransfer {
    lba: u64,
    blocks: u32,
    bytes: u64,
}

/// Why a logical-block transfer cannot be encoded or safely received.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "the transfer contains no logical blocks",
            Self::TooManyBlocks => "the transfer does not fit the nlb field",
            Self::BlockSize => "the logical-block size is zero",
            Self::OutsideNamespace => "the transfer crosses the namespace boundary",
            Self::LengthOverflow => "the transfer byte length overflows",
            Self::BufferTooSmall => "the dma buffer is shorter than the transfer",
            Self::ControllerLimit => "the transfer exceeds the proved controller limit",
        })
    }
}

impl core::error::Error for TransferError {}

impl ReadTransfer {
    /// Validate one read in command order: count, namespace extent, byte extent, then capacity.
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
