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

pub const IOSQES: u32 = 6;

pub const IOCQES: u32 = 4;

pub const MAX_TRANSFER_BLOCKS: u32 = u16::MAX as u32 + 1;

pub const SUPPORTED_BLOCK_BYTES: u32 = 512;

const NAMESPACE_SIZE_BYTES: usize = 8;
const LBA_FORMAT_BYTES: usize = 4;
const MAX_LBA_FORMATS: u8 = 16;
const SUPPORTED_BLOCK_SHIFT: u8 = 9;
const IDENTIFY_TRANSFER_BYTES: u64 = 4096;

mod identify {
    pub(super) const FORMAT_COUNT: usize = 25;
    pub(super) const FORMATTED: usize = 26;
    pub(super) const PROTECTION_SETTINGS: usize = 29;
    pub(super) const LBA_FORMATS: usize = 128;
}

pub const NAMESPACE_IDENTIFY_BYTES: usize =
    identify::LBA_FORMATS + MAX_LBA_FORMATS as usize * LBA_FORMAT_BYTES;

// `close` frees no DMA memory; only a completion or verified quiescence returns it.
#[derive(PartialEq, Eq, Debug)]
pub struct DmaState {
    state: DmaOwnership,
}

// The MMIO adapter consumes this proof while ringing the matching submission doorbell.
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

    pub fn reserve(&mut self) -> bool {
        if self.state != DmaOwnership::Idle {
            return false;
        }
        self.state = DmaOwnership::Reserved;
        true
    }

    pub fn cancel_unpublished(&mut self) -> bool {
        if self.state != DmaOwnership::Reserved {
            return false;
        }
        self.state = DmaOwnership::Idle;
        true
    }

    pub fn publish_with<T>(&mut self, handoff: impl FnOnce(DmaPublication<'_>) -> T) -> Option<T> {
        if self.state != DmaOwnership::Reserved {
            return None;
        }
        self.state = DmaOwnership::InFlight;
        Some(handoff(DmaPublication { _state: self }))
    }

    #[must_use]
    pub fn complete(&mut self) -> bool {
        if self.state != DmaOwnership::InFlight {
            return false;
        }
        self.state = DmaOwnership::Idle;
        true
    }

    pub fn close(&mut self) {
        self.state = DmaOwnership::Closed;
    }

    pub const fn is_in_flight(&self) -> bool {
        matches!(self.state, DmaOwnership::InFlight)
    }
}

// Base 2.2 §3.1.
pub mod register {
    pub const CAP: usize = 0x00;
    pub const INTMS: usize = 0x0C;
    pub const CC: usize = 0x14;
    pub const CSTS: usize = 0x1C;
    pub const AQA: usize = 0x24;
    pub const ASQ: usize = 0x28;
    pub const ACQ: usize = 0x30;
    pub const DOORBELLS: usize = 0x1000;
}

pub mod configuration {
    pub const ENABLE: u32 = 1;
    pub const PAGE_SIZE_SHIFT: u32 = 7;
    pub const SUBMISSION_ENTRY_SHIFT: u32 = 16;
    pub const COMPLETION_ENTRY_SHIFT: u32 = 20;
}

pub mod status {
    pub const READY: u32 = 1;
    pub const FATAL: u32 = 1 << 1;
}

// These prove nothing about where they were read, so hardware authority must keep the mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DisabledRegisters {
    cc: u32,
    csts: u32,
}

impl DisabledRegisters {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControllerStateError {
    Active { cc: u32, csts: u32 },
}

impl fmt::Display for ControllerStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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

#[derive(Debug)]
#[must_use = "unverified controller ownership must be advanced or retained"]
pub struct ControllerPreparation<T> {
    owner: T,
}

#[derive(Debug)]
#[must_use = "disabled controller ownership must be retired or advanced"]
pub struct DisabledPreparation<T> {
    owner: T,
    _observed: DisabledRegisters,
}

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
    pub fn into_parts(self) -> (E, P) {
        (self.reason, self.preparation)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capabilities {
    bits: u64,
}

impl Capabilities {
    pub const fn from_bits(bits: u64) -> Self {
        Self { bits }
    }

    // `MQES` holds one less than the number of entries.
    pub const fn max_queue_entries(self) -> u32 {
        (self.bits as u32 & 0xFFFF) + 1
    }

    // `TO` counts units of 500 milliseconds.
    pub const fn timeout_ms(self) -> u64 {
        ((self.bits >> 24) & 0xFF) * 500
    }

    // `DSTRD` is the doorbell stride as `log2(bytes) - 2`.
    const fn doorbell_stride(self) -> u32 {
        ((self.bits >> 32) & 0xF) as u32
    }

    pub const fn doorbell_bytes(self) -> usize {
        4usize << self.doorbell_stride()
    }

    // `MPSMIN` and `MPSMAX` hold `log2(size) - 12`.
    const fn min_page_shift(self) -> u32 {
        (((self.bits >> 48) & 0xF) as u32) + 12
    }

    const fn max_page_shift(self) -> u32 {
        (((self.bits >> 52) & 0xF) as u32) + 12
    }

    pub const fn supports_page_size(self, bytes: u64) -> bool {
        let shift = bytes.trailing_zeros();
        bytes.is_power_of_two() && shift >= self.min_page_shift() && shift <= self.max_page_shift()
    }

    // `CAP` bit 37 is `CSS` bit 0, the NVM command set.
    pub const fn supports_nvm_command_set(self) -> bool {
        self.bits & (1 << 37) != 0
    }
}

// One entry always stays unused because equal head and tail means empty.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capacity {
    entries: u32,
}

impl Capacity {
    const MIN_ENTRIES: u32 = 2;

    const MAX_ENTRIES: u32 = 65536;

    pub const fn new(entries: u32) -> Option<Self> {
        if entries < Self::MIN_ENTRIES || entries > Self::MAX_ENTRIES {
            return None;
        }
        Some(Self { entries })
    }

    pub const fn entries(self) -> u32 {
        self.entries
    }

    pub const fn as_zero_based(self) -> u32 {
        self.entries - 1
    }

    pub const fn submission_bytes(self) -> u64 {
        self.entries as u64 * SUBMISSION_ENTRY_BYTES as u64
    }

    pub const fn completion_bytes(self) -> u64 {
        self.entries as u64 * COMPLETION_ENTRY_BYTES as u64
    }

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

// Doorbells are write-only, so the host shadows the tail it last published.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Doorbell {
    Submission,
    Completion,
}

// Base 2.2 §3.1.24.
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

    pub const fn number_of_queues(id: u16, submission: u16, completion: u16) -> Option<Self> {
        if submission == 0 || completion == 0 {
            return None;
        }
        let mut command = Self::empty(admin::SET_FEATURES, id);
        command.words[10] = feature::NUMBER_OF_QUEUES;
        command.words[11] = ((completion as u32 - 1) << 16) | (submission as u32 - 1);
        Some(command)
    }

    const fn with_namespace(mut self, namespace: u32) -> Self {
        self.words[1] = namespace;
        self
    }

    // Every transfer is one contiguous extent, so PRP2 stays the zero `empty` wrote.
    const fn with_buffer(mut self, address: u64) -> Self {
        self.words[6] = address as u32;
        self.words[7] = (address >> 32) as u32;
        self
    }

    const fn contiguous_extent(address: u64, length: u64, page_size: u64) -> bool {
        page_size >= 4096
            && page_size.is_power_of_two()
            && address.is_multiple_of(page_size)
            && length != 0
            && address.checked_add(length).is_some()
    }

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Completion {
    pub submission_head: u16,
    pub submission_queue: u16,
    pub id: u16,
    pub phase: bool,
    pub status: u16,
}

impl Completion {
    pub const fn from_bytes(bytes: [u8; COMPLETION_ENTRY_BYTES]) -> Self {
        let dword2 = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let dword3 = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        Self {
            submission_head: dword2 as u16,
            submission_queue: (dword2 >> 16) as u16,
            id: dword3 as u16,
            phase: (dword3 >> 16) & 1 != 0,
            status: (dword3 >> 17) as u16,
        }
    }

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Namespace {
    blocks: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NamespaceError {
    Empty,
    TooManyFormats(u16),
    FormatOutsideList { selected: u8, formats: u8 },
    ReservedFormatBits(u8),
    ReservedLbaFormatBits(u8),
    Metadata(u16),
    Protection(u8),
    BlockSize(u8),
}

impl fmt::Display for NamespaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the namespace has no logical blocks"),
            Self::TooManyFormats(formats) => {
                write!(formatter, "the namespace reports {formats} lba formats")
            }
            Self::FormatOutsideList { selected, formats } => write!(
                formatter,
                "lba format {selected} is outside the {formats} reported formats"
            ),
            Self::ReservedFormatBits(bits) => {
                write!(
                    formatter,
                    "formatted lba size has reserved bits {bits:#04x}"
                )
            }
            Self::ReservedLbaFormatBits(bits) => {
                write!(
                    formatter,
                    "the selected lba format has reserved bits {bits:#04x}"
                )
            }
            Self::Metadata(bytes) => {
                write!(
                    formatter,
                    "the selected lba format has {bytes} metadata bytes"
                )
            }
            Self::Protection(bits) => {
                write!(
                    formatter,
                    "the namespace enables protection settings {bits:#04x}"
                )
            }
            Self::BlockSize(shift) => {
                write!(formatter, "the selected lba data size exponent is {shift}")
            }
        }
    }
}

impl core::error::Error for NamespaceError {}

impl Namespace {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReadTransfer {
    lba: u64,
    blocks: u32,
    bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransferError {
    Empty,
    TooManyBlocks,
    BlockSize,
    OutsideNamespace,
    LengthOverflow,
    BufferTooSmall,
    ControllerLimit,
}

impl fmt::Display for TransferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
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

    pub const fn within_limit(self, limit: u64) -> Result<Self, TransferError> {
        if self.bytes > limit {
            return Err(TransferError::ControllerLimit);
        }
        Ok(self)
    }

    pub const fn lba(self) -> u64 {
        self.lba
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    const fn zero_based_blocks(self) -> u16 {
        (self.blocks - 1) as u16
    }
}

#[cfg(test)]
mod tests;
