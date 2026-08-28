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
//! // A controller reporting MQES = 63 has queues of 64 entries...
//! let cap = Capacity::from_mqes(63).expect("a real queue size");
//! assert_eq!(cap.entries(), 64);
//!
//! // ...and can have 63 commands outstanding, never 64.
//! assert_eq!(cap.outstanding_limit(), 63);
//!
//! assert_eq!(register::CAP, 0x00);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

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
/// The mode is chosen once. After calibration, code generation and processor
/// speed cannot change the real duration; before calibration, a caller still
/// has a finite, explicit fallback rather than an accidental endless wait
/// (D-200).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollLimit {
    /// Stop at this monotonic nanosecond.
    Deadline(u64),
    /// Permit exactly this many more hardware reads.
    Attempts(u32),
}

impl PollLimit {
    /// Choose a monotonic deadline when `now_ns` exists, or `fallback_attempts`
    /// while the boot clock is unavailable.
    pub const fn after_ms(now_ns: Option<u64>, timeout_ms: u64, fallback_attempts: u32) -> Self {
        match now_ns {
            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),
            None => Self::Attempts(fallback_attempts),
        }
    }

    /// Whether one more hardware read is permitted at `now_ns`.
    ///
    /// A monotonic limit does not spend attempts: any number of fast polls
    /// sees the same real deadline. An attempt limit ignores a clock which
    /// appears later, so boot behaviour cannot change mode halfway through a
    /// transaction.
    pub const fn permit(&mut self, now_ns: Option<u64>) -> bool {
        match self {
            Self::Deadline(deadline) => match now_ns {
                Some(now) => now < *deadline,
                None => false,
            },
            Self::Attempts(remaining) if *remaining > 0 => {
                *remaining -= 1;
                true
            }
            Self::Attempts(_) => false,
        }
    }
}

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

/// Identify Namespace prefix through all sixteen negotiated LBA formats.
pub const NAMESPACE_IDENTIFY_BYTES: usize =
    LBA_FORMATS_OFFSET + MAX_LBA_FORMATS as usize * LBA_FORMAT_BYTES;

/// Whether memory named by the current command may return to its allocator.
///
/// A completion restores ordinary ownership. An uncompleted command does not:
/// only a verified controller stop makes its memory recyclable, and a failed
/// stop makes the retained state permanent even if a late completion appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaState {
    state: DmaOwnership,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DmaOwnership {
    Idle,
    InFlight,
    Quiesced,
    Retained,
}

impl DmaState {
    /// No command has made memory device-owned yet.
    pub const fn new() -> Self {
        Self {
            state: DmaOwnership::Idle,
        }
    }

    /// Record publication of one command.
    ///
    /// False after another command was published or the controller was
    /// stopped, so the state cannot silently forget an older DMA owner.
    pub fn begin(&mut self) -> bool {
        if self.state != DmaOwnership::Idle {
            return false;
        }
        self.state = DmaOwnership::InFlight;
        true
    }

    /// Record the matching completion, if a command is still in flight.
    pub fn complete(&mut self) {
        if self.state == DmaOwnership::InFlight {
            self.state = DmaOwnership::Idle;
        }
    }

    /// End access by stopping the controller, or retain every reachable run.
    pub fn quiesce(&mut self, verified: bool) {
        if matches!(self.state, DmaOwnership::Quiesced | DmaOwnership::Retained) {
            return;
        }
        self.state = if verified {
            DmaOwnership::Quiesced
        } else {
            DmaOwnership::Retained
        };
    }

    /// Whether another command may be published.
    pub const fn may_submit(self) -> bool {
        matches!(self.state, DmaOwnership::Idle)
    }

    /// Whether every device access has been settled.
    pub const fn is_settled(self) -> bool {
        matches!(self.state, DmaOwnership::Quiesced | DmaOwnership::Retained)
    }

    /// Whether a controller stop was observed rather than assumed.
    pub const fn is_quiesced(self) -> bool {
        matches!(self.state, DmaOwnership::Quiesced)
    }

    /// Whether a submitted command still lacks a matching completion.
    pub const fn is_in_flight(self) -> bool {
        matches!(self.state, DmaOwnership::InFlight)
    }

    /// Whether memory reachable by this controller must never be recycled.
    pub const fn must_retain(self) -> bool {
        matches!(self.state, DmaOwnership::Retained)
    }

    /// Whether DMA memory can be recycled now.
    pub const fn may_recycle(self) -> bool {
        matches!(self.state, DmaOwnership::Idle | DmaOwnership::Quiesced)
    }
}

impl Default for DmaState {
    fn default() -> Self {
        Self::new()
    }
}

/// Byte offsets of the controller registers, from Base 2.2 §3.1.
pub mod register {
    /// Controller capabilities. 64 bits, and **read, never assumed** — `012`
    /// §11 records that VMware's values are unmeasured.
    pub const CAP: usize = 0x00;
    /// Version.
    pub const VS: usize = 0x08;
    /// Interrupt mask set.
    pub const INTMS: usize = 0x0C;
    /// Interrupt mask clear.
    pub const INTMC: usize = 0x10;
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
    /// Shift of `CSS`, the command set.
    pub const COMMAND_SET_SHIFT: u32 = 4;
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

    /// `CQR`: whether queues must be physically contiguous.
    ///
    /// This driver only builds contiguous queues, so a controller that
    /// requires them is satisfied and one that does not is unaffected. It is
    /// read so that the *next* driver, which might not, has the answer.
    pub const fn requires_contiguous_queues(self) -> bool {
        self.bits & (1 << 16) != 0
    }

    /// `TO`: the timeout, in units of 500 milliseconds.
    ///
    /// How long the controller may take to become ready. A driver that waits
    /// a fixed time instead is a driver that works on one machine.
    pub const fn timeout_ms(self) -> u64 {
        ((self.bits >> 24) & 0xFF) * 500
    }

    /// `DSTRD`: the doorbell stride, as `log2(bytes) - 2`.
    pub const fn doorbell_stride(self) -> u32 {
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
    pub const fn min_page_shift(self) -> u32 {
        (((self.bits >> 48) & 0xF) as u32) + 12
    }

    /// `MPSMAX`: the largest, likewise.
    pub const fn max_page_shift(self) -> u32 {
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

    /// Build a capacity from what `CAP.MQES` reported, which is one less.
    pub const fn from_mqes(mqes: u32) -> Option<Self> {
        match mqes.checked_add(1) {
            Some(entries) => Self::new(entries),
            None => None,
        }
    }

    /// Entries in the queue.
    pub const fn entries(self) -> u32 {
        self.entries
    }

    /// What goes in `AQA`, and in the create-queue command: one less.
    pub const fn as_zero_based(self) -> u32 {
        self.entries - 1
    }

    /// Commands that may be outstanding at once.
    ///
    /// **One less than the entries**, always. This is the whole of the rule.
    pub const fn outstanding_limit(self) -> u32 {
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
    pub const fn next(self, index: u32) -> u32 {
        if index + 1 == self.entries {
            0
        } else {
            index + 1
        }
    }

    /// Whether a queue with this head and tail can take another entry.
    ///
    /// The `size - 1` rule applied: the tail may advance to one short of the
    /// head and no further, because equal means empty.
    pub const fn has_room(self, head: u32, tail: u32) -> bool {
        self.next(tail) != head
    }

    /// How many entries are occupied.
    pub const fn occupied(self, head: u32, tail: u32) -> u32 {
        if tail >= head {
            tail - head
        } else {
            self.entries - head + tail
        }
    }
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
/// use nvme::{Capabilities, doorbell};
///
/// // A controller with the usual stride of zero: four bytes apart.
/// let cap = Capabilities::from_bits(0);
/// assert_eq!(doorbell(cap, 0, false), Some(0x1000)); // admin submission
/// assert_eq!(doorbell(cap, 0, true), Some(0x1004));  // admin completion
/// assert_eq!(doorbell(cap, 1, false), Some(0x1008)); // i/o submission
/// ```
pub const fn doorbell(cap: Capabilities, queue: u16, completion: bool) -> Option<usize> {
    let index = (queue as usize) * 2 + if completion { 1 } else { 0 };
    match index.checked_mul(cap.doorbell_bytes()) {
        Some(offset) => offset.checked_add(register::DOORBELLS),
        None => None,
    }
}

/// Admin command opcodes this driver uses.
pub mod admin {
    /// Create an I/O submission queue.
    pub const CREATE_SUBMISSION_QUEUE: u8 = 0x01;
    /// Create an I/O completion queue.
    pub const CREATE_COMPLETION_QUEUE: u8 = 0x05;
    /// Identify.
    pub const IDENTIFY: u8 = 0x06;
}

/// NVM command opcodes this driver uses.
pub mod nvm {
    /// Read.
    pub const READ: u8 = 0x02;
}

/// One submission entry, built field by field.
///
/// Sixty-four bytes, and the layout is the specification's. Built through this
/// type rather than by writing offsets at the call site, so that a field that
/// moved would move once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    words: [u32; SUBMISSION_ENTRY_BYTES / 4],
}

impl Command {
    /// An empty command with an opcode and an identifier.
    ///
    /// The identifier is what a completion is matched by — **not** the slot,
    /// because a controller may complete out of order and does.
    pub const fn new(opcode: u8, id: u16) -> Self {
        let mut words = [0u32; SUBMISSION_ENTRY_BYTES / 4];
        words[0] = opcode as u32 | ((id as u32) << 16);
        Self { words }
    }

    /// Set the namespace this command is about.
    pub const fn namespace(mut self, nsid: u32) -> Self {
        self.words[1] = nsid;
        self
    }

    /// Set the two physical region pointers.
    pub const fn buffers(mut self, prp1: u64, prp2: u64) -> Self {
        self.words[6] = prp1 as u32;
        self.words[7] = (prp1 >> 32) as u32;
        self.words[8] = prp2 as u32;
        self.words[9] = (prp2 >> 32) as u32;
        self
    }

    /// Set one of the six command-specific dwords, `cdw10` through `cdw15`.
    ///
    /// # Panics
    ///
    /// If `which` is not in `10..=15`, which is a caller naming a dword that
    /// is not command-specific.
    #[must_use]
    pub const fn dword(mut self, which: usize, value: u32) -> Self {
        assert!(which >= 10 && which <= 15, "cdw10 through cdw15 only");
        self.words[which] = value;
        self
    }

    /// The command's identifier.
    pub const fn id(self) -> u16 {
        (self.words[0] >> 16) as u16
    }

    /// The command's opcode.
    pub const fn opcode(self) -> u8 {
        self.words[0] as u8
    }

    /// The sixty-four bytes to write into the queue.
    pub fn to_bytes(self) -> [u8; SUBMISSION_ENTRY_BYTES] {
        let mut bytes = [0u8; SUBMISSION_ENTRY_BYTES];
        let mut index = 0;
        while index < self.words.len() {
            let word = self.words[index].to_le_bytes();
            bytes[index * 4] = word[0];
            bytes[index * 4 + 1] = word[1];
            bytes[index * 4 + 2] = word[2];
            bytes[index * 4 + 3] = word[3];
            index += 1;
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
}

impl core::fmt::Display for TransferError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Empty => "the transfer contains no logical blocks",
            Self::TooManyBlocks => "the transfer does not fit the nlb field",
            Self::BlockSize => "the logical-block size is zero",
            Self::OutsideNamespace => "the transfer crosses the namespace boundary",
            Self::LengthOverflow => "the transfer byte length overflows",
            Self::BufferTooSmall => "the dma buffer is shorter than the transfer",
        })
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

/// Where the data for a transfer lives, as the controller wants it told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regions {
    /// The first pointer: the start of the buffer, page offset included.
    pub prp1: u64,
    /// The second: zero, one more page, or the address of a list.
    pub prp2: u64,
    /// How many entries a list at `prp2` must hold. Zero when `prp2` is not a
    /// list.
    pub list_entries: usize,
}

/// Build the physical region pointers for a transfer.
///
/// The rules from Base 2.2 §4.1, and each one is a way to get it wrong:
///
/// - `PRP1` carries the **offset within the page**; every later pointer must be
///   page-aligned.
/// - If what remains after the first page's tail is nothing, `PRP2` is zero.
/// - If it is one page or less, `PRP2` is that page — **not** a list of one.
/// - Otherwise `PRP2` is the address of a list, and the list's entries are the
///   pages after the first.
///
/// # Errors
///
/// `None` if the buffer is empty, if the transfer would run past the end of the
/// address space, or if a list would be needed and `list` is `None`.
///
/// # Examples
///
/// ```
/// use nvme::regions;
///
/// // Half a page from the start of one: it all fits in PRP1.
/// let one = regions(0x1000, 2048, 4096, None).expect("a valid transfer");
/// assert_eq!((one.prp1, one.prp2, one.list_entries), (0x1000, 0, 0));
///
/// // Straddling into a second page: PRP2 is that page, not a list.
/// let two = regions(0x1800, 4096, 4096, None).expect("a valid transfer");
/// assert_eq!((two.prp1, two.prp2, two.list_entries), (0x1800, 0x2000, 0));
/// ```
pub const fn regions(
    address: u64,
    length: u64,
    page_size: u64,
    list: Option<u64>,
) -> Option<Regions> {
    if length == 0 || page_size == 0 || !page_size.is_power_of_two() {
        return None;
    }
    let Some(end) = address.checked_add(length) else {
        return None;
    };

    let offset = address & (page_size - 1);
    let first = page_size - offset;
    if length <= first {
        return Some(Regions {
            prp1: address,
            prp2: 0,
            list_entries: 0,
        });
    }

    let second = address + first;
    let remaining = length - first;
    if remaining <= page_size {
        return Some(Regions {
            prp1: address,
            prp2: second,
            list_entries: 0,
        });
    }

    // Every page from the second to the last, which is what a list holds.
    let last = end - 1;
    let pages = ((last / page_size) - (second / page_size) + 1) as usize;
    match list {
        Some(at) => Some(Regions {
            prp1: address,
            prp2: at,
            list_entries: pages,
        }),
        None => None,
    }
}

/// The physical address a PRP list's `index`th entry names.
///
/// The pages after the first, in order. Separate from [`regions`] because the
/// list is written into memory the caller owns and this is the arithmetic for
/// filling it.
///
/// # Errors
///
/// `None` past the end of the transfer.
pub const fn list_entry(address: u64, length: u64, page_size: u64, index: usize) -> Option<u64> {
    if length == 0 || page_size == 0 || !page_size.is_power_of_two() {
        return None;
    }
    let offset = address & (page_size - 1);
    let first = page_size - offset;
    if length <= first {
        return None;
    }
    let second = address + first;
    let Some(page) = (index as u64).checked_mul(page_size) else {
        return None;
    };
    let Some(at) = second.checked_add(page) else {
        return None;
    };
    if at >= address + length {
        return None;
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_monotonic_poll_limit_expires_by_time_not_iteration_rate() {
        let mut limit = PollLimit::after_ms(Some(1_000_000_000), 500, 1);
        for _ in 0..10_000 {
            assert!(limit.permit(Some(1_499_999_999)));
        }
        assert!(!limit.permit(Some(1_500_000_000)));
        assert!(!limit.permit(None), "a vanished clock fails closed");
    }

    #[test]
    fn a_clockless_poll_limit_permits_its_exact_fallback_count() {
        let mut limit = PollLimit::after_ms(None, u64::MAX, 3);
        assert!(limit.permit(None));
        assert!(limit.permit(Some(u64::MAX)));
        assert!(limit.permit(None));
        assert!(!limit.permit(None));
    }

    #[test]
    fn a_deadline_saturates_instead_of_wrapping_into_the_past() {
        let mut limit = PollLimit::after_ms(Some(u64::MAX - 1), u64::MAX, 0);
        assert!(limit.permit(Some(u64::MAX - 1)));
        assert!(!limit.permit(Some(u64::MAX)));
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

    struct DelayedDevice {
        answered: bool,
        stopped: bool,
    }

    impl DelayedDevice {
        fn poll(&self, state: &mut DmaState) {
            if self.answered {
                state.complete();
            }
        }

        fn stop(&self, state: &mut DmaState) {
            state.quiesce(self.stopped);
        }
    }

    /// A late device can never make memory recyclable after stop was unproven.
    #[test]
    fn delayed_completion_cannot_recycle_unquiesced_dma_memory() {
        let mut uncertain = DmaState::new();
        assert!(uncertain.begin());
        let mut device = DelayedDevice {
            answered: false,
            stopped: false,
        };
        device.poll(&mut uncertain);
        assert!(!uncertain.may_recycle());
        device.stop(&mut uncertain);
        assert!(uncertain.must_retain());
        assert!(!uncertain.may_recycle());

        device.answered = true;
        device.poll(&mut uncertain);
        assert!(
            uncertain.must_retain(),
            "a late completion cannot undo retention"
        );
        assert!(!uncertain.may_recycle());

        let mut stopped = DmaState::new();
        assert!(stopped.begin());
        DelayedDevice {
            answered: false,
            stopped: true,
        }
        .stop(&mut stopped);
        assert!(stopped.is_quiesced());
        assert!(stopped.may_recycle());

        let mut completed = DmaState::new();
        assert!(completed.begin());
        completed.complete();
        assert!(completed.may_submit());
        assert!(completed.may_recycle());
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
            assert_eq!(capacity.outstanding_limit(), entries - 1);
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
                let room = capacity.has_room(head, tail);
                let would_wrap_onto_the_head = capacity.next(tail) == head;
                assert_eq!(room, !would_wrap_onto_the_head, "head {head}, tail {tail}");
            }
        }

        // And the count that follows from it: three, never four.
        let mut tail = 0;
        let mut admitted = 0;
        while capacity.has_room(0, tail) {
            tail = capacity.next(tail);
            admitted += 1;
        }
        assert_eq!(admitted, capacity.outstanding_limit());
        assert_eq!(admitted, 3);
    }

    /// A queue's occupancy is the distance from head to tail, wrapping.
    #[test]
    fn occupancy_wraps_with_the_queue() {
        let capacity = Capacity::new(8).expect("eight entries");
        assert_eq!(capacity.occupied(0, 0), 0);
        assert_eq!(capacity.occupied(0, 5), 5);
        assert_eq!(capacity.occupied(5, 0), 3);
        assert_eq!(capacity.occupied(6, 4), 6);
        assert_eq!(
            capacity.occupied(1, 0),
            7,
            "the full queue is size minus one"
        );
    }

    /// `MQES` is one less than the size, which is the specification's own
    /// off-by-one and the first place to get it wrong.
    #[test]
    fn the_reported_maximum_is_one_less_than_the_size() {
        assert_eq!(Capacity::from_mqes(1).expect("two entries").entries(), 2);
        assert_eq!(Capacity::from_mqes(63).expect("sixty-four").entries(), 64);
        assert_eq!(
            Capacity::from_mqes(65535).expect("the largest").entries(),
            65536
        );
        // A controller reporting zero has queues of one entry, which hold
        // nothing: refused rather than accepted and hung on.
        assert!(Capacity::from_mqes(0).is_none());
        assert!(Capacity::from_mqes(u32::MAX).is_none());
    }

    /// Every capability field is read from its own bits.
    ///
    /// Pairwise distinct values so no two fields can be swapped with this
    /// still passing (D-136), and every one of them changes what a correct
    /// driver does.
    #[test]
    fn every_capability_field_is_read_from_its_own_bits() {
        // MQES 0x00FF, CQR set, TO 0x0A, DSTRD 0x3, NVM command set,
        // MPSMIN 1, MPSMAX 5.
        let bits = 0x0000_0000_0000_00FFu64
            | (1 << 16)
            | (0x0A << 24)
            | (0x3 << 32)
            | (1 << 37)
            | (1 << 48)
            | (5 << 52);
        let cap = Capabilities::from_bits(bits);

        assert_eq!(cap.max_queue_entries(), 256);
        assert!(cap.requires_contiguous_queues());
        assert_eq!(cap.timeout_ms(), 5_000);
        assert_eq!(cap.doorbell_stride(), 3);
        assert_eq!(cap.doorbell_bytes(), 32);
        assert!(cap.supports_nvm_command_set());
        assert_eq!(cap.min_page_shift(), 13);
        assert_eq!(cap.max_page_shift(), 17);

        // A page size outside the reported range is refused, which is what
        // stops a driver programming `CC.MPS` with something the controller
        // will not accept and then waiting for a ready bit that never comes.
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
        assert!(Capacity::from_mqes(0).is_none());
    }

    /// Doorbells alternate, and the stride is the controller's.
    ///
    /// A driver that assumes four bytes works on every controller reporting a
    /// stride of zero and silently writes a *different queue's* doorbell on
    /// any that does not.
    #[test]
    fn doorbells_alternate_and_use_the_controllers_stride() {
        let narrow = Capabilities::from_bits(0);
        assert_eq!(doorbell(narrow, 0, false), Some(0x1000));
        assert_eq!(doorbell(narrow, 0, true), Some(0x1004));
        assert_eq!(doorbell(narrow, 1, false), Some(0x1008));
        assert_eq!(doorbell(narrow, 1, true), Some(0x100C));

        // Stride 2: sixteen bytes apart, so the I/O submission doorbell is at
        // 0x1020 and a driver assuming four would write the admin pair.
        let wide = Capabilities::from_bits(2 << 32);
        assert_eq!(wide.doorbell_bytes(), 16);
        assert_eq!(doorbell(wide, 0, false), Some(0x1000));
        assert_eq!(doorbell(wide, 1, false), Some(0x1020));
        assert_eq!(doorbell(wide, 1, true), Some(0x1030));
    }

    /// A command puts every field where the specification says.
    #[test]
    fn a_command_puts_every_field_where_the_specification_says() {
        let command = Command::new(nvm::READ, 0xABCD)
            .namespace(1)
            .buffers(0x1122_3344_5566_7788, 0x99AA_BBCC_DDEE_FF00)
            .dword(10, 0x0000_0064)
            .dword(12, 0x0000_0007);

        assert_eq!(command.opcode(), 0x02);
        assert_eq!(command.id(), 0xABCD);

        let bytes = command.to_bytes();
        assert_eq!(bytes.len(), 64);
        // CDW0: opcode in the low byte, identifier in the high half.
        assert_eq!(&bytes[0..4], &[0x02, 0x00, 0xCD, 0xAB]);
        // NSID.
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
        // PRP1 at 24, PRP2 at 32 — the two most consequential offsets here.
        assert_eq!(&bytes[24..32], &0x1122_3344_5566_7788u64.to_le_bytes());
        assert_eq!(&bytes[32..40], &0x99AA_BBCC_DDEE_FF00u64.to_le_bytes());
        // CDW10 at 40 and CDW12 at 48.
        assert_eq!(&bytes[40..44], &0x64u32.to_le_bytes());
        assert_eq!(&bytes[48..52], &7u32.to_le_bytes());
        // And CDW11, which was not set, is zero rather than borrowed.
        assert_eq!(&bytes[44..48], &[0, 0, 0, 0]);
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

    /// A transfer inside one page needs one pointer.
    #[test]
    fn a_transfer_inside_one_page_needs_one_pointer() {
        for (address, length) in [
            (0x1000u64, 4096u64),
            (0x1000, 1),
            (0x1FFF, 1),
            (0x1800, 2048),
        ] {
            let built = regions(address, length, 4096, None).expect("a valid transfer");
            assert_eq!(built.prp1, address, "PRP1 carries the offset");
            assert_eq!(built.prp2, 0);
            assert_eq!(built.list_entries, 0);
        }
    }

    /// A transfer over two pages needs two pointers and no list.
    ///
    /// The case a driver most often gets wrong by reaching for a list: the
    /// specification says `PRP2` *is* the second page when there is only one
    /// more, and a list of one is not the same thing.
    #[test]
    fn a_transfer_over_two_pages_needs_no_list() {
        let built = regions(0x1800, 4096, 4096, None).expect("a valid transfer");
        assert_eq!(built.prp1, 0x1800);
        assert_eq!(built.prp2, 0x2000, "the next page, not a list");
        assert_eq!(built.list_entries, 0);

        // Exactly two pages from an aligned start is the boundary: the second
        // page is `PRP2` and there is still no list.
        let aligned = regions(0x1000, 8192, 4096, None).expect("a valid transfer");
        assert_eq!(aligned.prp2, 0x2000);
        assert_eq!(aligned.list_entries, 0);

        // One byte more needs a list, and refuses without somewhere to put it.
        assert!(regions(0x1000, 8193, 4096, None).is_none());
    }

    /// A longer transfer names a list, and the list holds the pages after the
    /// first.
    #[test]
    fn a_longer_transfer_names_a_list_of_the_pages_after_the_first() {
        let built = regions(0x1800, 16384, 4096, Some(0x9000)).expect("a valid transfer");
        assert_eq!(built.prp1, 0x1800);
        assert_eq!(built.prp2, 0x9000, "the list");
        // 2048 in the first page, then 0x2000, 0x3000, 0x4000, 0x5000.
        assert_eq!(built.list_entries, 4);

        for (index, expected) in [(0, 0x2000u64), (1, 0x3000), (2, 0x4000), (3, 0x5000)] {
            assert_eq!(
                list_entry(0x1800, 16384, 4096, index),
                Some(expected),
                "entry {index}"
            );
        }
        // And nothing past the end.
        assert_eq!(list_entry(0x1800, 16384, 4096, 4), None);
    }

    /// A transfer that cannot be described is refused rather than truncated.
    #[test]
    fn a_transfer_that_cannot_be_described_is_refused() {
        assert!(regions(0x1000, 0, 4096, None).is_none(), "empty");
        assert!(regions(0x1000, 4096, 0, None).is_none(), "no page size");
        assert!(
            regions(0x1000, 4096, 5000, None).is_none(),
            "not a power of two"
        );
        assert!(regions(u64::MAX, 2, 4096, None).is_none(), "past the end");
        assert!(
            list_entry(0x1000, 1024, 4096, 0).is_none(),
            "no second page"
        );
    }
}
