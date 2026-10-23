//! PCI configuration-space addressing and BAR decoding.
//!
//! `wost` owns the CF8/CFC instructions; this crate owns their host-tested
//! arithmetic. Offsets are dword-aligned, BAR masks are complemented in their
//! register width, and 64-bit BARs consume two slots. CF8/CFC is the common
//! mechanism on the QEMU i440fx and VMware 440BX targets (`012` §2).
//!
//! # Examples
//!
//! ```
//! use pci_config::{Address, header};
//!
//! // Bus 0, device 2, function 0, the vendor and device word.
//! let at = Address::new(0, 2, 0, header::VENDOR).expect("a real address");
//! assert_eq!(at.bits(), 0x8000_1000);
//!
//! // The enable bit is what makes CF8 a configuration cycle rather than a
//! // write to whatever else lives at that port.
//! assert!(at.bits() & (1 << 31) != 0);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::sync::atomic::{AtomicU64, Ordering};

/// Devices on one bus.
pub const DEVICES: u8 = 32;

/// Functions in one device.
pub const FUNCTIONS: u8 = 8;

/// Bytes of the configuration header this addressing reaches.
///
/// The original mechanism sees 256; the extended space ECAM adds is not
/// reachable through CF8/CFC at all, and a driver that needs it needs ECAM.
pub const HEADER_BYTES: u16 = 256;

/// Base address registers a type 0 header has.
pub const BARS: usize = 6;

/// Base address registers a type 1 header has.
///
/// The third slot in a bridge header is its bus-number register, not a BAR.
pub const BRIDGE_BARS: usize = 2;

/// Byte offsets in the configuration header.
///
/// Type 0 (a device) and type 1 (a bridge) share everything up to `0x0F`, and
/// the header type at `0x0E` is what says which the rest is.
pub mod header {
    /// Vendor id, and device id in the high half. `0xFFFF` means nothing is
    /// there — the bus returns all ones for an absent function.
    pub const VENDOR: u16 = 0x00;
    /// Command register. Bit 0 is I/O space, bit 1 is memory space, and bit 2
    /// is bus master.
    pub const COMMAND: u16 = 0x04;
    /// Revision in the low byte, then prog-if, subclass and class.
    pub const CLASS: u16 = 0x08;
    /// Cache line size, latency, header type, BIST.
    pub const HEADER_TYPE: u16 = 0x0C;
    /// The first base address register.
    pub const BAR0: u16 = 0x10;
    /// A bridge's primary, secondary and subordinate bus numbers.
    pub const BUS_NUMBERS: u16 = 0x18;
}

/// Command-register bits this kernel writes.
pub mod command {
    /// Respond to I/O-space accesses.
    pub const IO_SPACE: u16 = 1;
    /// Respond to memory-space accesses.
    pub const MEMORY_SPACE: u16 = 1 << 1;
    /// **Bus master.** Without it the controller cannot DMA, queues never
    /// advance, and there is no device error to read (`012` §2).
    pub const BUS_MASTER: u16 = 1 << 2;
}

const PROBE_DECODE: u16 = command::IO_SPACE | command::MEMORY_SPACE;
const PROBE_DISABLED: u16 = PROBE_DECODE | command::BUS_MASTER;

/// A configuration-space address, ready for the address port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address(u32);

impl Address {
    /// Bit 31: this is a configuration cycle and not something else.
    const ENABLE: u32 = 1 << 31;

    /// Build an address, or refuse one that cannot be encoded.
    ///
    /// # Errors
    ///
    /// `None` for a device above 31, a function above 7, an offset at or past
    /// [`HEADER_BYTES`], or an offset that is not a multiple of four — the
    /// last because configuration space is read a dword at a time and a
    /// misaligned offset does not fault, it quietly reads the dword containing
    /// it.
    pub const fn new(bus: u8, device: u8, function: u8, offset: u16) -> Option<Self> {
        if device >= DEVICES || function >= FUNCTIONS {
            return None;
        }
        if offset >= HEADER_BYTES || !offset.is_multiple_of(4) {
            return None;
        }
        Some(Self(
            Self::ENABLE
                | ((bus as u32) << 16)
                | ((device as u32) << 11)
                | ((function as u32) << 8)
                | offset as u32,
        ))
    }

    /// The word to write to the address port.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The same function at a different offset.
    ///
    /// The common move — walk a header — expressed so the bus, device and
    /// function are not re-encoded and cannot be re-encoded differently.
    pub const fn at(self, offset: u16) -> Option<Self> {
        if offset >= HEADER_BYTES || !offset.is_multiple_of(4) {
            return None;
        }
        Some(Self((self.0 & !0xFF) | offset as u32))
    }

    /// The identity of this function, with no register offset attached.
    ///
    /// Device ownership is a bus/device/function claim. Treating two offsets
    /// in the same header as different identities would let two drivers claim
    /// the same function.
    pub const fn function_base(self) -> Self {
        Self(self.0 & !0xFF)
    }

    /// Which bus this names.
    pub const fn bus(self) -> u8 {
        (self.0 >> 16) as u8
    }

    /// Which device on that bus.
    pub const fn device(self) -> u8 {
        ((self.0 >> 11) & 0x1F) as u8
    }

    /// Which function of that device.
    pub const fn function(self) -> u8 {
        ((self.0 >> 8) & 0x07) as u8
    }
}

/// The exclusive right to mutate one PCI function.
///
/// Its registry identity, generation and address are opaque. A token minted by
/// another registry therefore cannot authorize a hardware adapter backed by
/// this one, even when both name the same bus/device/function tuple.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "dropping function authority permanently retains its claim"]
pub struct FunctionAuthority {
    lease: FunctionLease,
    registers: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FunctionLease {
    registry: u64,
    at: Address,
    generation: u64,
}

impl FunctionAuthority {
    /// The claimed bus/device/function identity, with register offset zero.
    pub const fn address(&self) -> Address {
        self.lease.at
    }

    /// BAR register slots proved by the claimed function's header layout.
    pub const fn bar_registers(&self) -> usize {
        self.registers
    }
}

/// A bounded registry of exclusive function authorities.
#[derive(Debug, PartialEq, Eq)]
pub struct FunctionClaims<const N: usize> {
    slots: [Option<FunctionLease>; N],
    mapped_bars: [u8; N],
    identity: u64,
    generation: u64,
    commands: CommandLeases<N>,
}

static NEXT_FUNCTION_REGISTRY: AtomicU64 = AtomicU64::new(1);

impl<const N: usize> FunctionClaims<N> {
    /// Create an empty function-authority registry.
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            mapped_bars: [0; N],
            identity: 0,
            generation: 0,
            commands: CommandLeases::new(),
        }
    }

    /// Claim one function whose header layout was read from that function.
    ///
    /// The returned token is bound to this registry, a non-repeating
    /// generation and the canonical function address.
    ///
    /// # Errors
    ///
    /// Refuses unsupported header layouts, a duplicate function, a full
    /// registry or exhausted non-repeating identities before changing a slot.
    pub fn claim(
        &mut self,
        at: Address,
        header: HeaderType,
    ) -> Result<FunctionAuthority, FunctionClaimError> {
        let Some(registers) = header.bar_registers() else {
            return Err(FunctionClaimError::Header(header.layout()));
        };
        let at = at.function_base();
        if self.is_claimed(at) {
            return Err(FunctionClaimError::Taken);
        }
        let Some(room) = self.slots.iter().position(Option::is_none) else {
            return Err(FunctionClaimError::NoRoom);
        };
        let registry = if self.identity == 0 {
            let identity = NEXT_FUNCTION_REGISTRY
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current.checked_add(1)
                })
                .map_err(|_| FunctionClaimError::Exhausted)?;
            self.identity = identity;
            identity
        } else {
            self.identity
        };
        let Some(generation) = self.generation.checked_add(1) else {
            return Err(FunctionClaimError::Exhausted);
        };
        self.generation = generation;
        let lease = FunctionLease {
            registry,
            at,
            generation,
        };
        self.slots[room] = Some(lease);
        self.mapped_bars[room] = 0;
        Ok(FunctionAuthority { lease, registers })
    }

    /// Whether `authority` is this registry's exact live generation.
    pub fn holds(&self, authority: &FunctionAuthority) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|held| *held == authority.lease)
    }

    /// Whether any register offset of `at`'s function is held.
    pub fn is_claimed(&self, at: Address) -> bool {
        let at = at.function_base();
        self.slots.iter().flatten().any(|held| held.at == at)
    }

    /// Release one exact function authority.
    ///
    /// # Errors
    ///
    /// Returns the unchanged token if it is foreign or stale.
    fn release(&mut self, authority: FunctionAuthority) -> Result<(), FunctionAuthority> {
        let Some(index) = self
            .slots
            .iter()
            .position(|held| *held == Some(authority.lease))
        else {
            return Err(authority);
        };
        if self.mapped_bars.get(index).copied() != Some(0) {
            return Err(authority);
        }
        let Some(slot) = self.slots.get_mut(index) else {
            return Err(authority);
        };
        *slot = None;
        Ok(())
    }

    /// Functions recorded so far.
    #[expect(
        clippy::len_without_is_empty,
        reason = "the authority registry exposes a diagnostic count, not a collection API"
    )]
    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }
}

impl<const N: usize> Default for FunctionClaims<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Why exclusive function authority could not be minted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FunctionClaimError {
    /// Another authority already holds this function.
    Taken,
    /// The registry's fixed lifetime bound is exhausted.
    NoRoom,
    /// Registry identity or generation space is exhausted.
    Exhausted,
    /// The header layout is neither a device nor a bridge.
    Header(u8),
}

impl core::fmt::Display for FunctionClaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Taken => core::fmt::Write::write_str(f, "the function is already claimed"),
            Self::NoRoom => core::fmt::Write::write_str(f, "the function registry is full"),
            Self::Exhausted => {
                core::fmt::Write::write_str(f, "function claim generations are exhausted")
            }
            Self::Header(layout) => write!(f, "header layout {layout} is unsupported"),
        }
    }
}

impl core::error::Error for FunctionClaimError {}

/// What a function says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Class {
    /// The broad kind: `0x01` is mass storage, `0x06` a bridge.
    pub class: u8,
    /// Within that kind: `0x08` under mass storage is a non-volatile memory
    /// controller.
    pub subclass: u8,
    /// The programming interface: `0x02` under `01:08` is NVM Express.
    pub interface: u8,
    /// Silicon revision, which decides nothing here and is read because it
    /// shares the word.
    pub revision: u8,
}

impl Class {
    /// Decode the class word at [`header::CLASS`].
    pub const fn from_bits(bits: u32) -> Self {
        Self {
            revision: bits as u8,
            interface: (bits >> 8) as u8,
            subclass: (bits >> 16) as u8,
            class: (bits >> 24) as u8,
        }
    }

    /// Whether this is an NVM Express controller.
    ///
    /// All three bytes, not two: `01:08` is "non-volatile memory controller"
    /// and `01:08:01` is NVMHCI, a different and older interface this driver
    /// would misprogram.
    pub const fn is_nvme(self) -> bool {
        self.class == 0x01 && self.subclass == 0x08 && self.interface == 0x02
    }

    /// Whether this is a PCI-to-PCI bridge, and so a bus to descend into.
    ///
    /// **Enumeration must recurse.** VMware places functions behind
    /// `pciBridge0`–`pciBridge7`, so a scan of bus zero alone may never find
    /// the controller (`012` §2).
    pub const fn is_bridge(self) -> bool {
        self.class == 0x06 && self.subclass == 0x04
    }
}

/// What the header-type byte says about a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderType(u8);

impl HeaderType {
    /// Decode the byte at [`header::HEADER_TYPE`] + 2.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Whether the device has functions past zero.
    ///
    /// Bit 7. A walk that ignores it and probes all eight anyway is not wrong,
    /// only slower; a walk that *reads* it and gets the bit wrong misses every
    /// function of a multi-function device.
    pub const fn is_multifunction(self) -> bool {
        self.0 & 0x80 != 0
    }

    /// The layout of the rest of the header: 0 a device, 1 a bridge.
    pub const fn layout(self) -> u8 {
        self.0 & 0x7F
    }

    /// Whether the rest of the header is a bridge's.
    pub const fn is_bridge(self) -> bool {
        self.layout() == 1
    }

    /// BAR register slots defined by this header layout.
    ///
    /// `None` for layouts this kernel cannot safely size: their registers do
    /// not have either the type-0 device or type-1 bridge meaning.
    pub const fn bar_registers(self) -> Option<usize> {
        match self.layout() {
            0 => Some(BARS),
            1 => Some(BRIDGE_BARS),
            _ => None,
        }
    }
}

/// A base address register, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    /// A memory window.
    Memory {
        /// Where it begins.
        address: u64,
        /// Whether it took two register slots.
        wide: bool,
        /// Whether the device tolerates speculative reads. **Not** whether the
        /// kernel should map it that way: a register file is mapped
        /// uncacheable whatever this says, and mapping it write-back is a
        /// silent, hard-to-debug bug (`012` §2).
        prefetchable: bool,
    },
    /// An I/O port window.
    Io {
        /// The first port.
        port: u16,
    },
    /// The slot is unused, or is the high half of the register before it.
    Absent,
}

impl Bar {
    /// Bit 0: zero for memory, one for I/O.
    const IS_IO: u32 = 1;
    /// Bits 2:1 select the memory BAR encoding.
    const TYPE_MASK: u32 = 0b110;
    /// The obsolete memory encoding constrained below one megabyte.
    const BELOW_ONE_MEGABYTE: u32 = 0b010;
    /// Bits 2:1 == 0b10: the address is 64 bits and takes the next slot too.
    const WIDE: u32 = 0b100;
    /// Bits 2:1 == 0b11 have no defined memory BAR meaning.
    const RESERVED: u32 = 0b110;
    /// Bit 3.
    const PREFETCHABLE: u32 = 0b1000;

    /// Decode one supported slot, given the slot after it when it is wide.
    ///
    /// `high` is read unconditionally by the caller because it must be read
    /// *before* this can say whether it was needed, and reading it costs one
    /// configuration cycle.
    ///
    /// # Errors
    ///
    /// `None` for the unsupported below-one-megabyte memory encoding or the
    /// reserved memory encoding.
    pub const fn decode(low: u32, high: u32) -> Option<Self> {
        if low == 0 {
            return Some(Self::Absent);
        }
        if low & Self::IS_IO != 0 {
            return Some(Self::Io {
                port: (low & !0x3) as u16,
            });
        }
        let memory_type = low & Self::TYPE_MASK;
        if memory_type == Self::BELOW_ONE_MEGABYTE {
            return None;
        }
        if memory_type == Self::RESERVED {
            return None;
        }
        let base = (low & !0xF) as u64;
        if memory_type == Self::WIDE {
            Some(Self::Memory {
                address: base | ((high as u64) << 32),
                wide: true,
                prefetchable: low & Self::PREFETCHABLE != 0,
            })
        } else {
            Some(Self::Memory {
                address: base,
                wide: false,
                prefetchable: low & Self::PREFETCHABLE != 0,
            })
        }
    }
}

/// Register width used to interpret a probed memory-window mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryWidth {
    /// One 32-bit base-address register.
    Bits32,
    /// A low/high pair forming one 64-bit base-address register.
    Bits64,
}

/// The size of a **memory** window, from the all-ones probe.
///
/// The size is the two's complement of the returned address mask. `width`
/// keeps complementation in the register's actual width (D-168).
///
/// # Errors
///
/// `None` when the mask decodes no window.
///
/// # Examples
///
/// ```
/// use pci_config::{MemoryWidth, memory_window_size};
///
/// // A 16 KiB window: the low fourteen address bits are not decoded.
/// assert_eq!(
///     memory_window_size(0xFFFF_C000, 0, MemoryWidth::Bits32),
///     Some(0x4000),
/// );
///
/// // A 64-bit window of four gigabytes, where the high half carries it.
/// assert_eq!(
///     memory_window_size(0, 0xFFFF_FFFF, MemoryWidth::Bits64),
///     Some(0x1_0000_0000),
/// );
/// ```
pub const fn memory_window_size(low: u32, high: u32, width: MemoryWidth) -> Option<u64> {
    match width {
        MemoryWidth::Bits64 => {
            let mask = ((low & !0xF) as u64) | ((high as u64) << 32);
            if mask == 0 {
                return None;
            }
            let size = (!mask).wrapping_add(1);
            if size == 0 { None } else { Some(size) }
        }
        MemoryWidth::Bits32 => {
            let mask = low & !0xF;
            if mask == 0 {
                return None;
            }
            Some((!mask).wrapping_add(1) as u64)
        }
    }
}

/// The size of an **I/O** window, from the same probe.
///
/// I/O masks stay 32-bit and must describe a run within the 16-bit port space.
///
/// # Errors
///
/// `None` when the mask decodes nothing, or when the size will not fit the
/// port space it claims to describe.
///
const fn io_window_size(mask: u32) -> Option<u16> {
    let mask = mask & !0x3;
    if mask == 0 {
        return None;
    }
    let size = (!mask).wrapping_add(1);
    // A run that does not fit the sixteen-bit port space is a device
    // misreporting, and claiming it would be claiming most of I/O space.
    if size >> u16::BITS == 0 {
        Some(size as u16)
    } else {
        None
    }
}

/// One command-register access in a verified enable transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAccess {
    /// Read the command register. Only its low sixteen bits are consumed.
    Read,
    /// Write only command bits, leaving the status half of the dword clear.
    Write(u16),
}

/// One configuration-space operation in the entry-time DMA fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffAccess {
    /// Read the complete vendor/device identity dword.
    ReadIdentity,
    /// Read the complete revision/programming-interface/subclass/class dword.
    ReadClass,
    /// Read the command register; only its low sixteen bits are consumed.
    ReadCommand,
    /// Write command bits with a zero status half.
    WriteCommand(u16),
    /// Master-clear and mask every channel of both legacy 8237 controllers,
    /// then read their status registers: DMA1 in bits 0..8, DMA2 in 8..16.
    QuiesceIsaDma,
}

/// Request bits of both 8237 status registers in a [`HandoffAccess::QuiesceIsaDma`]
/// word; the terminal-count bits beside them clear on read and prove nothing.
pub const ISA_DMA_PENDING_REQUESTS: u32 = 0xf0f0;

/// Function authorities the platform PCI registry and kernel can hold at once.
///
/// A unit-wide VT-d fence claims every live requester the unit governs. The
/// default VMware machine alone presents 42, 32 of them root ports, so the
/// bound is sized by the qualified platform rather than by the test machine.
pub const FUNCTION_AUTHORITY_LIMIT: usize = 64;

/// Platform-bound policy for the entry-time DMA fence.
///
/// The VMware profile recognizes one exact emulated 440BX host bridge whose
/// command bit 2 is a read-only chipset control rather than a PCI requester
/// bus-master enable, and one exact PIIX4 ISA bridge whose read-only bit 2 does
/// govern a requester: its 8237 controllers. That requester stays in every
/// inventory and is quiesced at the controllers instead. The caller must select
/// the profile only from requester-inaccessible VMware CPU evidence; every
/// other function and every near miss remains under the strict BME-off rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffFencePolicy {
    kind: HandoffFencePolicyKind,
}

/// Exact platform evidence that one present function is not a DMA requester.
///
/// Construction is private to [`HandoffFencePolicy`], so topology code cannot
/// remove an arbitrary function from the requester inventory.
#[derive(Debug)]
#[must_use = "qualified non-requester evidence must constrain requester inventory"]
pub struct QualifiedNonRequester {
    at: Address,
    identity: u32,
    class: u32,
    command: u16,
}

/// Exact platform evidence that one requester's bus mastering is held off at
/// its DMA engines because its command bit 2 cannot be cleared.
///
/// Construction is private to the entry fence, after the exact VMware PIIX4
/// profile matched and both 8237 controllers were masked with no request
/// pending. The function remains a requester in every inventory.
#[derive(Debug)]
#[must_use = "quiesced requester evidence must reach the topology snapshot"]
pub struct QuiescedIsaRequester {
    at: Address,
    identity: u32,
    class: u32,
    command: u16,
}

impl QuiescedIsaRequester {
    /// Exact function whose entry-time quiesce minted this authority.
    pub const fn address(&self) -> Address {
        self.at
    }
}

/// The sealed snapshot's record of the one entry-quiesced ISA requester.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuiescedRequester {
    at: Address,
    identity: u32,
    command: u16,
}

impl QuiescedRequester {
    /// Exact function the entry fence quiesced.
    pub const fn address(&self) -> Address {
        self.at
    }

    /// Identity a later fence must still read.
    pub const fn identity(&self) -> u32 {
        self.identity
    }

    /// Command word a later fence must still read.
    pub const fn command(&self) -> u16 {
        self.command
    }
}

/// One untrusted later observation checked against non-requester authority.
///
/// Construction proves nothing by itself. [`PciTopology::observe_non_requester`]
/// consumes the earlier entry-fence authority and compares every security-
/// relevant field before excluding the function from requester inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionObservation {
    at: Address,
    identity: u32,
    class_bits: u32,
    command: u16,
    class: Class,
    header: HeaderType,
    bus_numbers: Option<u32>,
}

impl FunctionObservation {
    /// Gather the independently read fields of one present function.
    pub const fn new(
        at: Address,
        identity: u32,
        class_bits: u32,
        command: u16,
        class: Class,
        header: HeaderType,
        bus_numbers: Option<u32>,
    ) -> Self {
        Self {
            at,
            identity,
            class_bits,
            command,
            class,
            header,
            bus_numbers,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandoffFencePolicyKind {
    Strict,
    Vmware440Bx,
}

impl HandoffFencePolicy {
    /// Require every present PCI function to read back with BME clear.
    pub const fn strict() -> Self {
        Self {
            kind: HandoffFencePolicyKind::Strict,
        }
    }

    /// Admit only VMware's exact non-requesting 440BX host-bridge command.
    pub const fn vmware_440bx() -> Self {
        Self {
            kind: HandoffFencePolicyKind::Vmware440Bx,
        }
    }

    /// Classify the one exact platform function allowed to retain command bit 2.
    fn classify_non_requester(
        self,
        at: Address,
        identity: u32,
        class: u32,
        before: u16,
        found: u16,
    ) -> Option<QualifiedNonRequester> {
        const VMWARE_440BX_IDENTITY: u32 = 0x7190_8086;
        const HOST_BRIDGE_CLASS: u32 = 0x06_00_00_00;
        const CLASS_WITHOUT_REVISION: u32 = 0xffff_ff00;
        const VMWARE_HOST_COMMAND: u16 = command::MEMORY_SPACE | command::BUS_MASTER;

        (matches!(self.kind, HandoffFencePolicyKind::Vmware440Bx)
            && at.bus() == 0
            && at.device() == 0
            && at.function() == 0
            && identity == VMWARE_440BX_IDENTITY
            && class & CLASS_WITHOUT_REVISION == HOST_BRIDGE_CLASS
            && before == VMWARE_HOST_COMMAND
            && found == VMWARE_HOST_COMMAND)
            .then_some(QualifiedNonRequester {
                at: at.function_base(),
                identity,
                class,
                command: found,
            })
    }

    /// Whether this policy may quiesce an ISA requester at its 8237 controllers.
    pub const fn quiesces_isa_dma(self) -> bool {
        matches!(self.kind, HandoffFencePolicyKind::Vmware440Bx)
    }

    /// Whether `at` is VMware's exact PIIX4 ISA bridge with read-only BME.
    fn classify_isa_requester(
        self,
        at: Address,
        identity: u32,
        class: u32,
        before: u16,
        found: u16,
    ) -> bool {
        const VMWARE_PIIX4_ISA_IDENTITY: u32 = 0x7110_8086;
        const ISA_BRIDGE_CLASS: u32 = 0x06_01_00_00;
        const ISA_CLASS_MASK: u32 = 0xffff_ff00;
        const VMWARE_ISA_COMMAND: u16 =
            command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER;

        self.quiesces_isa_dma()
            && (at.bus(), at.device(), at.function()) == (0, 7, 0)
            && identity == VMWARE_PIIX4_ISA_IDENTITY
            && class & ISA_CLASS_MASK == ISA_BRIDGE_CLASS
            && before == VMWARE_ISA_COMMAND
            && found == VMWARE_ISA_COMMAND
    }
}

impl QualifiedNonRequester {
    /// Exact function whose entry-time fence minted this authority.
    pub const fn address(&self) -> Address {
        self.at
    }
}

/// What an exhaustive entry-time DMA fence observed.
#[derive(Debug)]
pub struct HandoffFenceReport {
    functions: u32,
    disabled: u32,
    retained_non_requesters: u32,
    retained_non_requester: Option<QualifiedNonRequester>,
    quiesced_requesters: u32,
    quiesced_requester: Option<QuiescedIsaRequester>,
}

impl HandoffFenceReport {
    /// Requesters held off at their 8237 controllers because BME is read-only.
    pub const fn quiesced_requesters(&self) -> u32 {
        self.quiesced_requesters
    }

    /// Remove the exact entry-quiesce authority for the later topology snapshot.
    pub fn take_quiesced_requester(&mut self) -> Option<QuiescedIsaRequester> {
        self.quiesced_requester.take()
    }

    /// Present functions safely classified by the platform-bound fence.
    pub const fn functions(&self) -> u32 {
        self.functions
    }

    /// Functions whose inherited BME bit the fence cleared.
    pub const fn disabled(&self) -> u32 {
        self.disabled
    }

    /// Qualified non-requester host bridges whose read-only bit 2 remained set.
    pub const fn retained_non_requesters(&self) -> u32 {
        self.retained_non_requesters
    }

    /// Remove the exact entry-fence authority for the later topology snapshot.
    pub fn take_non_requester(&mut self) -> Option<QualifiedNonRequester> {
        self.retained_non_requester.take()
    }
}

/// Why the entry-time requester fence could not prove DMA quiescence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HandoffFenceError {
    /// One required configuration-space cycle was unavailable.
    Access {
        /// Function whose configuration cycle failed.
        at: Address,
        /// Operation that could not complete.
        operation: HandoffAccess,
    },
    /// BME clear did not preserve and read back the exact command word.
    CommandRefused {
        /// Function whose command word refused the fence.
        at: Address,
        /// Stable vendor/device identity observed around the transaction.
        identity: u32,
        /// Stable class dword observed around the transaction.
        class: u32,
        /// Exact BME-off word written or required.
        expected: u16,
        /// Word returned by the readback.
        found: u16,
    },
    /// The quiesced ISA requester still showed a DMA request after masking.
    IsaDmaPending {
        /// Function whose 8237 controllers were masked.
        at: Address,
        /// Combined status word: DMA1 in bits 0..8, DMA2 in 8..16.
        status: u32,
    },
    /// A present function changed identity during its fence.
    IdentityChanged {
        /// Function whose identity changed.
        at: Address,
        /// Identity read before the command transaction.
        expected: u32,
        /// Identity read after the command transaction.
        found: u32,
    },
    /// A present function changed class during its fence.
    ClassChanged {
        /// Function whose class changed.
        at: Address,
        /// Class dword read before the command transaction.
        expected: u32,
        /// Class dword read after the command transaction.
        found: u32,
    },
}

impl core::fmt::Display for HandoffFenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Access { at, operation } => write!(
                f,
                "configuration {operation:?} failed for {:02x}:{:02x}.{}",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::CommandRefused {
                at,
                identity,
                class,
                expected,
                found,
            } => write!(
                f,
                "{:02x}:{:02x}.{} identity {identity:#010x} class {class:#010x} command read back as {found:#06x}, expected {expected:#06x}",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::IsaDmaPending { at, status } => write!(
                f,
                "{:02x}:{:02x}.{} 8237 status {status:#06x} still requests DMA after masking",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::IdentityChanged {
                at,
                expected,
                found,
            } => write!(
                f,
                "{:02x}:{:02x}.{} identity changed from {expected:#010x} to {found:#010x}",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::ClassChanged {
                at,
                expected,
                found,
            } => write!(
                f,
                "{:02x}:{:02x}.{} class changed from {expected:#010x} to {found:#010x}",
                at.bus(),
                at.device(),
                at.function()
            ),
        }
    }
}

impl core::error::Error for HandoffFenceError {}

/// Clear and verify BME on every present function in the legacy PCI segment.
///
/// Every bus, device and function number is probed independently; no header
/// hint or bridge route can shrink the inventory. The caller must already own
/// the one configuration mechanism used by `access` and hold trusted platform
/// authority that its visibility and requester identities remain static.
///
/// # Errors
///
/// Returns the exact unavailable cycle, refused command readback or changing
/// function identity. A refusal never restores BME on a function already
/// fenced.
pub fn fence_dma_handoff(
    policy: HandoffFencePolicy,
    mut access: impl FnMut(Address, HandoffAccess) -> Option<u32>,
) -> Result<HandoffFenceReport, HandoffFenceError> {
    let mut functions = 0u32;
    let mut disabled = 0u32;
    let mut retained_non_requesters = 0u32;
    let mut retained_non_requester = None;
    let mut quiesced_requesters = 0u32;
    let mut quiesced_requester = None;
    for bus in 0..=u8::MAX {
        for device in 0..DEVICES {
            for function in 0..FUNCTIONS {
                let Some(at) = Address::new(bus, device, function, header::VENDOR) else {
                    continue;
                };
                let identity =
                    access(at, HandoffAccess::ReadIdentity).ok_or(HandoffFenceError::Access {
                        at,
                        operation: HandoffAccess::ReadIdentity,
                    })?;
                if is_absent(identity as u16) {
                    continue;
                }
                let Some(command_at) = at.at(header::COMMAND) else {
                    continue;
                };
                let before = access(command_at, HandoffAccess::ReadCommand).ok_or(
                    HandoffFenceError::Access {
                        at,
                        operation: HandoffAccess::ReadCommand,
                    },
                )? as u16;
                let expected = before & !command::BUS_MASTER;
                if before != expected {
                    access(command_at, HandoffAccess::WriteCommand(expected)).ok_or(
                        HandoffFenceError::Access {
                            at,
                            operation: HandoffAccess::WriteCommand(expected),
                        },
                    )?;
                }
                let found = access(command_at, HandoffAccess::ReadCommand).ok_or(
                    HandoffFenceError::Access {
                        at,
                        operation: HandoffAccess::ReadCommand,
                    },
                )? as u16;
                let retained_class = if found != expected {
                    let Some(class_at) = at.at(header::CLASS) else {
                        continue;
                    };
                    let class = access(class_at, HandoffAccess::ReadClass).ok_or(
                        HandoffFenceError::Access {
                            at,
                            operation: HandoffAccess::ReadClass,
                        },
                    )?;
                    if let Some(authority) =
                        policy.classify_non_requester(at, identity, class, before, found)
                    {
                        Some((class_at, class, Retained::NonRequester(authority)))
                    } else if policy.classify_isa_requester(at, identity, class, before, found) {
                        let status = access(at, HandoffAccess::QuiesceIsaDma).ok_or(
                            HandoffFenceError::Access {
                                at,
                                operation: HandoffAccess::QuiesceIsaDma,
                            },
                        )?;
                        if status & ISA_DMA_PENDING_REQUESTS != 0 {
                            return Err(HandoffFenceError::IsaDmaPending { at, status });
                        }
                        let authority = QuiescedIsaRequester {
                            at: at.function_base(),
                            identity,
                            class,
                            command: found,
                        };
                        Some((class_at, class, Retained::Quiesced(authority)))
                    } else {
                        return Err(HandoffFenceError::CommandRefused {
                            at,
                            identity,
                            class,
                            expected,
                            found,
                        });
                    }
                } else if before != expected {
                    disabled += 1;
                    None
                } else {
                    None
                };
                let current =
                    access(at, HandoffAccess::ReadIdentity).ok_or(HandoffFenceError::Access {
                        at,
                        operation: HandoffAccess::ReadIdentity,
                    })?;
                if current != identity {
                    return Err(HandoffFenceError::IdentityChanged {
                        at,
                        expected: identity,
                        found: current,
                    });
                }
                if let Some((class_at, class, retained)) = retained_class {
                    let current_class = access(class_at, HandoffAccess::ReadClass).ok_or(
                        HandoffFenceError::Access {
                            at,
                            operation: HandoffAccess::ReadClass,
                        },
                    )?;
                    if current_class != class {
                        return Err(HandoffFenceError::ClassChanged {
                            at,
                            expected: class,
                            found: current_class,
                        });
                    }
                    match retained {
                        Retained::NonRequester(authority) => {
                            retained_non_requesters += 1;
                            retained_non_requester = Some(authority);
                        }
                        Retained::Quiesced(authority) => {
                            quiesced_requesters += 1;
                            quiesced_requester = Some(authority);
                        }
                    }
                }
                functions += 1;
            }
        }
    }
    Ok(HandoffFenceReport {
        functions,
        disabled,
        retained_non_requesters,
        retained_non_requester,
        quiesced_requesters,
        quiesced_requester,
    })
}

/// The exact authority a refused BME clear can still mint at entry.
enum Retained {
    NonRequester(QualifiedNonRequester),
    Quiesced(QuiescedIsaRequester),
}

/// The exclusive right to finish one function's command-register transition.
///
/// The function identity, original word, installed word and registry
/// generation are deliberately opaque. Restoration therefore cannot be aimed
/// at another function or replayed over a later lease.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "dropping command state permanently retains its function lease"]
pub struct CommandState {
    lease: CommandLease,
    before: u16,
    installed: u16,
    phase: CommandPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandLease {
    registry: u64,
    function: FunctionLease,
    generation: u64,
}

impl CommandLease {
    const fn command_address(self) -> Address {
        Address(self.function.at.0 | header::COMMAND as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandPhase {
    Installed,
    Restoring(u16),
}

#[derive(Debug, PartialEq, Eq)]
struct CommandLeases<const N: usize> {
    slots: [Option<CommandLease>; N],
    identity: u64,
    generation: u64,
}

static NEXT_COMMAND_REGISTRY: AtomicU64 = AtomicU64::new(1);

impl<const N: usize> CommandLeases<N> {
    const fn new() -> Self {
        Self {
            slots: [None; N],
            identity: 0,
            generation: 0,
        }
    }

    fn reserve(&mut self, authority: &FunctionAuthority) -> Result<CommandLease, EnableError> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|lease| lease.function == authority.lease)
        {
            return Err(EnableError::Leased);
        }
        let Some(room) = self.slots.iter().position(Option::is_none) else {
            return Err(EnableError::NoRoom);
        };
        let registry = if self.identity == 0 {
            let identity = NEXT_COMMAND_REGISTRY
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current.checked_add(1)
                })
                .map_err(|_| EnableError::Exhausted)?;
            self.identity = identity;
            identity
        } else {
            self.identity
        };
        let Some(generation) = self.generation.checked_add(1) else {
            return Err(EnableError::Exhausted);
        };
        self.generation = generation;
        let lease = CommandLease {
            registry,
            function: authority.lease,
            generation,
        };
        self.slots[room] = Some(lease);
        Ok(lease)
    }

    fn holds(&self, authority: &FunctionAuthority, lease: CommandLease) -> bool {
        self.holds_function(authority.lease, lease)
    }

    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {
        lease.function == function && self.slots.iter().flatten().any(|held| *held == lease)
    }

    fn release(&mut self, lease: CommandLease) -> bool {
        let Some(slot) = self.slots.iter_mut().find(|held| **held == Some(lease)) else {
            return false;
        };
        *slot = None;
        true
    }

    fn begin_probe(&mut self, authority: &FunctionAuthority) -> Result<CommandProbe, EnableError> {
        self.reserve(authority).map(CommandProbe)
    }

    /// Finish a BAR probe whose command word was restored exactly.
    ///
    /// # Errors
    ///
    /// Returns the unchanged probe token when it did not come from this
    /// registry or no longer names its live generation.
    fn finish_probe(
        &mut self,
        authority: &FunctionAuthority,
        probe: CommandProbe,
    ) -> Result<(), CommandProbe> {
        if self.holds(authority, probe.0) && self.release(probe.0) {
            Ok(())
        } else {
            Err(probe)
        }
    }

    fn stage_probe(
        &mut self,
        authority: &FunctionAuthority,
        probe: CommandProbe,
        before: u16,
    ) -> Result<CommandState, CommandProbe> {
        if !self.holds(authority, probe.0) {
            return Err(probe);
        }
        Ok(CommandState {
            lease: probe.0,
            before,
            installed: before & !PROBE_DISABLED,
            phase: CommandPhase::Installed,
        })
    }

    fn has_function(&self, authority: &FunctionAuthority) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|lease| lease.function == authority.lease)
    }
}

impl<const N: usize> Default for CommandLeases<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Why one function authority could not be returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FunctionReleaseError {
    /// A command transition or BAR restoration still owns this function.
    Active,
    /// The token is foreign to this function registry or no longer current.
    Stale,
}

impl core::fmt::Display for FunctionReleaseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Active => {
                core::fmt::Write::write_str(f, "the function still has a configuration sublease")
            }
            Self::Stale => {
                core::fmt::Write::write_str(f, "the function authority is foreign or stale")
            }
        }
    }
}

impl core::error::Error for FunctionReleaseError {}

/// A refused function release with unchanged retry authority.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a refused function release returns its authority for retry"]
pub struct FunctionReleaseFailure {
    reason: FunctionReleaseError,
    authority: FunctionAuthority,
}

impl FunctionReleaseFailure {
    /// Why release refused.
    pub const fn reason(&self) -> FunctionReleaseError {
        self.reason
    }

    /// Recover the exact function authority for retry.
    pub fn into_authority(self) -> FunctionAuthority {
        self.authority
    }
}

impl core::fmt::Display for FunctionReleaseFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for FunctionReleaseFailure {}

/// Release a function only after every command and probe sublease is gone.
///
/// # Errors
///
/// Returns the unchanged authority while a sublease remains or if the token
/// is foreign or stale. No registry slot changes on refusal.
pub fn release_function<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: FunctionAuthority,
) -> Result<(), FunctionReleaseFailure> {
    if !functions.holds(&authority) {
        return Err(FunctionReleaseFailure {
            reason: FunctionReleaseError::Stale,
            authority,
        });
    }
    if functions.commands.has_function(&authority) {
        return Err(FunctionReleaseFailure {
            reason: FunctionReleaseError::Active,
            authority,
        });
    }
    if functions
        .slot_of(authority.lease)
        .and_then(|slot| functions.mapped_bars.get(slot))
        .is_some_and(|mapped| *mapped != 0)
    {
        return Err(FunctionReleaseFailure {
            reason: FunctionReleaseError::Active,
            authority,
        });
    }
    functions
        .release(authority)
        .map_err(|authority| FunctionReleaseFailure {
            reason: FunctionReleaseError::Stale,
            authority,
        })
}

/// Exclusive command-register reservation held for one BAR probe.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "dropping a probe token permanently retains its function reservation"]
pub struct CommandProbe(CommandLease);

/// A refused command enablement, including cleanup authority when uncertain.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a refused command transition may retain restoration authority"]
pub struct EnableFailure {
    reason: EnableError,
    state: Option<CommandState>,
}

impl EnableFailure {
    /// Why enablement refused.
    pub const fn reason(&self) -> EnableError {
        self.reason
    }

    /// Recover an uncertain transition for exact restoration.
    pub fn into_state(self) -> Option<CommandState> {
        self.state
    }
}

impl From<EnableError> for EnableFailure {
    fn from(reason: EnableError) -> Self {
        Self {
            reason,
            state: None,
        }
    }
}

/// A refused command restoration with the unchanged retry state.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a refused command restoration returns its state for retry"]
pub struct RestoreFailure {
    reason: EnableError,
    state: CommandState,
}

impl RestoreFailure {
    /// Why the lease could not be completed.
    pub const fn reason(&self) -> EnableError {
        self.reason
    }

    /// Recover the exact command word for another restoration attempt.
    pub fn into_state(self) -> CommandState {
        self.state
    }
}

/// Why memory decode and bus mastering could not be verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnableError {
    /// This function already has a live command transition.
    Leased,
    /// Every bounded command-transition slot is occupied.
    NoRoom,
    /// The non-repeating command-transition generation was exhausted.
    Exhausted,
    /// The token does not name a live registry generation.
    Stale,
    /// The command word changed after this lease installed it.
    Changed(u16),
    /// A configuration access failed and any attempted change was restored.
    Access,
    /// The device read back a command other than the exact word written.
    Refused(u16),
    /// The original command could not be read back exactly after failure.
    Restore,
    /// The DMA domain offered as isolation names another function.
    ForeignDomain,
}

impl core::fmt::Display for EnableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Leased => {
                core::fmt::Write::write_str(f, "the function already has a command lease")
            }
            Self::NoRoom => core::fmt::Write::write_str(f, "the command-lease registry is full"),
            Self::Exhausted => {
                core::fmt::Write::write_str(f, "command-lease generations are exhausted")
            }
            Self::Stale => core::fmt::Write::write_str(f, "the command lease is no longer current"),
            Self::Changed(observed) => {
                write!(f, "the leased command register changed to {observed:#06x}")
            }
            Self::Access => core::fmt::Write::write_str(f, "configuration access failed"),
            Self::Refused(observed) => {
                write!(f, "the command register read back as {observed:#06x}")
            }
            Self::Restore => {
                core::fmt::Write::write_str(f, "the original command could not be verified")
            }
            Self::ForeignDomain => {
                core::fmt::Write::write_str(f, "the DMA domain isolates another function")
            }
        }
    }
}

impl core::error::Error for EnableError {}

/// Enable memory decode while explicitly withholding bus mastering.
///
/// This is the safe first stage for a memory-mapped controller whose firmware
/// DMA state must be disabled before bus mastering is admitted.
pub fn enable_memory_space<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    if !functions.holds(authority) {
        return Err(EnableFailure::from(EnableError::Stale));
    }
    enable_memory_space_with_leases(&mut functions.commands, authority, access)
}

/// Exclusively fence one requester's bus mastering for a unit-wide DMA
/// transition.
///
/// The returned command owner records every captured bit except BME as its
/// restoration target. It must therefore remain attached to the remapping
/// unit until the unit transition is complete; restoring it can never revive
/// unisolated DMA.
///
/// # Errors
///
/// See [`EnableError`]. An uncertain rollback returns its still-live
/// [`CommandState`] through [`EnableFailure`] for quarantine or exact retry.
pub fn fence_bus_master<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    if !functions.holds(authority) {
        return Err(EnableFailure::from(EnableError::Stale));
    }
    let mut state = begin_command(
        &mut functions.commands,
        authority,
        0,
        command::BUS_MASTER,
        access,
    )?;
    // `begin_command` verified the installed word exactly. From this point a
    // later restoration is deliberately safe rather than firmware-exact.
    state.before &= !command::BUS_MASTER;
    Ok(state)
}

/// Freshly verify that one live command owner still has BME clear.
///
/// This is the last selected-function check before a unit-wide VT-d write.
/// It neither changes nor consumes the command owner.
pub fn verify_bus_master_fence<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: &CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !functions.holds(authority)
        || !functions.commands.holds(authority, state.lease)
        || state.phase != CommandPhase::Installed
    {
        return Err(EnableError::Stale);
    }
    let current = access(state.lease.command_address(), CommandAccess::Read)
        .ok_or(EnableError::Access)? as u16;
    if current != state.installed {
        return Err(EnableError::Changed(current));
    }
    if current & command::BUS_MASTER != 0 {
        return Err(EnableError::Refused(current));
    }
    Ok(())
}

fn enable_memory_space_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    begin_command(
        leases,
        authority,
        command::MEMORY_SPACE,
        command::BUS_MASTER,
        access,
    )
}

#[cfg(test)]
fn enable_bus_master<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    if !functions.holds(authority) {
        return Err(EnableFailure::from(EnableError::Stale));
    }
    enable_bus_master_with_leases(&mut functions.commands, authority, access)
}

#[cfg(test)]
fn enable_bus_master_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    begin_command(
        leases,
        authority,
        command::MEMORY_SPACE | command::BUS_MASTER,
        0,
        access,
    )
}

fn begin_command<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    set: u16,
    clear: u16,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    let lease = leases.reserve(authority).map_err(|reason| EnableFailure {
        reason,
        state: None,
    })?;
    let at = lease.command_address();
    let before = match access(at, CommandAccess::Read) {
        Some(bits) => bits as u16,
        None => {
            assert!(
                leases.release(lease),
                "a pre-write command refusal lost its exact reservation"
            );
            return Err(EnableFailure {
                reason: EnableError::Access,
                state: None,
            });
        }
    };
    let installed = (before | set) & !clear;
    let mut state = CommandState {
        lease,
        before,
        installed,
        phase: CommandPhase::Restoring(before),
    };
    match change_command(at, before, installed, &mut access) {
        Ok(()) => {
            state.phase = CommandPhase::Installed;
            Ok(state)
        }
        Err(reason) if reason == EnableError::Restore => Err(EnableFailure {
            reason,
            state: Some(state),
        }),
        Err(reason) => {
            assert!(
                leases.release(lease),
                "a restored command refusal lost its exact reservation"
            );
            Err(EnableFailure {
                reason,
                state: None,
            })
        }
    }
}

fn change_command(
    at: Address,
    original: u16,
    wanted: u16,
    access: &mut impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if original == wanted {
        return Ok(());
    }

    let failure = match access(at, CommandAccess::Write(wanted)) {
        None => EnableError::Access,
        Some(_) => match access(at, CommandAccess::Read) {
            Some(bits) if bits as u16 == wanted => return Ok(()),
            Some(bits) => EnableError::Refused(bits as u16),
            None => EnableError::Access,
        },
    };

    let _ = access(at, CommandAccess::Write(original));
    match access(at, CommandAccess::Read) {
        Some(bits) if bits as u16 == original => Err(failure),
        Some(_) | None => Err(EnableError::Restore),
    }
}

#[cfg(test)]
fn admit_bus_master<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: &mut CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !functions.holds(authority) {
        return Err(EnableError::Stale);
    }
    admit_bus_master_with_leases(&mut functions.commands, authority.lease, state, access)
}

fn admit_bus_master_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    function: FunctionLease,
    state: &mut CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !leases.holds_function(function, state.lease) || state.phase != CommandPhase::Installed {
        return Err(EnableError::Stale);
    }
    let at = state.lease.command_address();
    let current = access(at, CommandAccess::Read).ok_or(EnableError::Access)? as u16;
    if current != state.installed {
        return Err(EnableError::Changed(current));
    }
    let wanted = current | command::MEMORY_SPACE | command::BUS_MASTER;
    state.phase = CommandPhase::Restoring(wanted);
    let result = change_command(at, current, wanted, &mut access);
    match result {
        Ok(()) => {
            state.installed = wanted;
            state.phase = CommandPhase::Installed;
            Ok(())
        }
        Err(error) => {
            if error != EnableError::Restore {
                state.phase = CommandPhase::Installed;
            }
            Err(error)
        }
    }
}

/// Permanently withhold bus mastering from a transient function lease.
///
/// The command word is read and the BME-clear result is verified while the
/// exact function and command generations remain live. Only a successful
/// readback removes BME from the word a later [`restore_command`] will write;
/// every other captured bit is still restored normally. An interrupted prior
/// command change is accepted only at one of its two recorded words, which
/// lets teardown settle an uncertain bus-master admission without inventing a
/// third state.
///
/// # Errors
///
/// Refuses stale authority, an independently changed word, configuration
/// access failure, or a transition whose exact result cannot be verified. The
/// caller retains `state` for retry or quarantine in every case.
pub fn disable_bus_master_for_retirement<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: &mut CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !functions.holds(authority) {
        return Err(EnableError::Stale);
    }
    disable_bus_master_for_retirement_with_leases(
        &mut functions.commands,
        authority.lease,
        state,
        access,
    )
}

fn disable_bus_master_for_retirement_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    function: FunctionLease,
    state: &mut CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !leases.holds_function(function, state.lease) {
        return Err(EnableError::Stale);
    }
    let at = state.lease.command_address();
    let current = access(at, CommandAccess::Read).ok_or(EnableError::Access)? as u16;
    let pending = match state.phase {
        CommandPhase::Installed => None,
        CommandPhase::Restoring(word) => Some(word),
    };
    if current != state.installed && pending != Some(current) {
        return Err(EnableError::Changed(current));
    }

    // `current` is now one of the command owner's exact recorded words. Make
    // it the rollback word before publishing the next possible hardware word,
    // so an interrupted retry still names both and only both possibilities.
    state.installed = current;
    let wanted = current & !command::BUS_MASTER;
    state.phase = CommandPhase::Restoring(wanted);
    match change_command(at, current, wanted, &mut access) {
        Ok(()) => {
            state.before &= !command::BUS_MASTER;
            state.installed = wanted;
            state.phase = CommandPhase::Installed;
            Ok(())
        }
        Err(EnableError::Restore) => Err(EnableError::Restore),
        Err(error) => {
            state.phase = CommandPhase::Installed;
            Err(error)
        }
    }
}

/// Restore the exact command word carried by a [`CommandState`].
///
/// # Errors
///
/// The unchanged state is returned in [`RestoreFailure`] whenever its registry
/// generation is stale, the installed word changed, an access failed, or the
/// final readback did not equal the captured word.
pub fn restore_command<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    if !functions.holds(authority) {
        return Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        });
    }
    restore_command_with_leases(&mut functions.commands, authority, state, access)
}

fn restore_command_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    state: CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    let mut state = state;
    if !leases.holds(authority, state.lease) {
        return Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        });
    }
    let at = state.lease.command_address();
    let current = match access(at, CommandAccess::Read) {
        Some(bits) => bits as u16,
        None => {
            return Err(RestoreFailure {
                reason: EnableError::Access,
                state,
            });
        }
    };
    if current == state.before {
        return if leases.release(state.lease) {
            Ok(())
        } else {
            Err(RestoreFailure {
                reason: EnableError::Stale,
                state,
            })
        };
    }
    let pending = match state.phase {
        CommandPhase::Installed => None,
        CommandPhase::Restoring(word) => Some(word),
    };
    if current != state.installed && pending != Some(current) {
        return Err(RestoreFailure {
            reason: EnableError::Changed(current),
            state,
        });
    }

    state.phase = CommandPhase::Restoring(current);
    let _ = access(at, CommandAccess::Write(state.before));
    match access(at, CommandAccess::Read) {
        Some(bits) if bits as u16 == state.before => {
            if leases.release(state.lease) {
                Ok(())
            } else {
                Err(RestoreFailure {
                    reason: EnableError::Stale,
                    state,
                })
            }
        }
        Some(bits) => Err(RestoreFailure {
            reason: EnableError::Refused(bits as u16),
            state,
        }),
        None => Err(RestoreFailure {
            reason: EnableError::Access,
            state,
        }),
    }
}

/// Finish a permanent command transition without restoring its old word.
///
/// # Errors
///
/// Returns the unchanged state unless its exact registry generation is live
/// and the command register still equals the installed word.
pub fn commit_command<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    if !functions.holds(authority) {
        return Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        });
    }
    commit_command_with_leases(&mut functions.commands, authority, state, access)
}

fn commit_command_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    state: CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    if !leases.holds(authority, state.lease) || state.phase != CommandPhase::Installed {
        return Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        });
    }
    let current = match access(state.lease.command_address(), CommandAccess::Read) {
        Some(bits) => bits as u16,
        None => {
            return Err(RestoreFailure {
                reason: EnableError::Access,
                state,
            });
        }
    };
    if current != state.installed {
        return Err(RestoreFailure {
            reason: EnableError::Changed(current),
            state,
        });
    }
    if leases.release(state.lease) {
        Ok(())
    } else {
        Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        })
    }
}

/// One configuration-space cycle in a BAR-probe transaction.
///
/// A platform adapter implements these against its configuration mechanism.
/// Keeping the sequence here makes its ordering and rollback host-testable;
/// the adapter only has to keep its hardware lock for the closure's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeAccess {
    /// Read the command register. Only its low sixteen bits are consumed.
    ReadCommand,
    /// Write only command bits. The status half of the dword must be zero.
    WriteCommand(u16),
    /// Read one BAR slot.
    ReadBar(usize),
    /// Write one BAR slot.
    WriteBar(usize, u32),
}

/// One decoded and sized BAR window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// What the register said.
    pub bar: Bar,
    /// How many bytes or ports it covers.
    pub size: u64,
}

/// Every window discovered by one BAR-restored probe transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbedBars {
    function: Option<FunctionLease>,
    windows: [Option<Window>; BARS],
}

/// A completed BAR probe whose safe command state remains exclusively owned.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "the staged command must be committed or restored before releasing its function"]
pub struct StagedProbe {
    bars: ProbedBars,
    command: CommandState,
}

impl StagedProbe {
    /// Separate the discovered windows from their command restoration owner.
    pub fn into_parts(self) -> (ProbedBars, CommandState) {
        (self.bars, self.command)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BarSlots {
    low: usize,
    high: Option<usize>,
}

impl ProbedBars {
    /// The decoded window in one BAR slot.
    pub fn window(&self, index: usize) -> Option<Window> {
        self.windows.get(index).copied().flatten()
    }

    /// All slots, including the empty high halves of wide BARs.
    pub const fn into_windows(self) -> [Option<Window>; BARS] {
        self.windows
    }
}

/// A live function's exclusive authority over one probed memory BAR.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a BAR authority must remain with its mapping or be returned"]
pub struct BarAuthority {
    function: FunctionLease,
    index: usize,
    window: Window,
}

impl BarAuthority {
    /// The BAR slot this authority names.
    pub const fn index(&self) -> usize {
        self.index
    }

    /// The exact decoded window observed by the restored probe.
    pub const fn window(&self) -> Window {
        self.window
    }
}

/// Why a probed memory BAR could not become an exclusive mapping authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BarAuthorityError {
    /// The function, registry generation, or probe provenance is not current.
    Stale,
    /// The BAR slot does not contain a memory window.
    NotMemory,
    /// Another live mapping authority already owns this BAR slot.
    Busy,
}

impl core::fmt::Display for BarAuthorityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Stale => core::fmt::Write::write_str(f, "the BAR authority is stale or foreign"),
            Self::NotMemory => core::fmt::Write::write_str(f, "the BAR is not a memory window"),
            Self::Busy => core::fmt::Write::write_str(f, "the BAR already has a mapping authority"),
        }
    }
}

impl core::error::Error for BarAuthorityError {}

/// A refused BAR-authority return with the unchanged token.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a refused BAR release returns its authority for retry"]
pub struct BarAuthorityFailure {
    reason: BarAuthorityError,
    authority: BarAuthority,
}

impl core::fmt::Display for BarAuthorityFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for BarAuthorityFailure {}

/// Why a live BAR no longer matches the probed identity behind its mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BarIdentityError {
    /// The function or BAR mapping generation is stale or foreign.
    Stale,
    /// Configuration space could not read the complete BAR identity.
    Access,
    /// The decoded BAR value changed since the restored probe.
    Changed,
}

impl core::fmt::Display for BarIdentityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Stale => core::fmt::Write::write_str(f, "the BAR identity is stale or foreign"),
            Self::Access => core::fmt::Write::write_str(f, "the BAR identity is unreadable"),
            Self::Changed => core::fmt::Write::write_str(f, "the BAR identity changed"),
        }
    }
}

impl core::error::Error for BarIdentityError {}

/// One access in the indivisible BAR-identity and memory-decode transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarEnableAccess {
    /// Read one BAR slot before any command write.
    ReadBar(usize),
    /// Access the command register through its verified transaction.
    Command(CommandAccess),
}

/// Why BAR-bound memory decoding could not be enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BarEnableError {
    /// The mapped BAR identity was stale, inaccessible, or changed.
    Identity(BarIdentityError),
    /// The command transaction refused or could not restore exactly.
    Command(EnableError),
}

impl core::fmt::Display for BarEnableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Identity(reason) => write!(f, "BAR identity: {reason}"),
            Self::Command(reason) => write!(f, "memory decode: {reason}"),
        }
    }
}

impl core::error::Error for BarEnableError {}

impl<const N: usize> FunctionClaims<N> {
    fn slot_of(&self, function: FunctionLease) -> Option<usize> {
        self.slots.iter().position(|held| *held == Some(function))
    }

    fn holds_bar(&self, authority: &BarAuthority) -> bool {
        let Some(slot) = self.slot_of(authority.function) else {
            return false;
        };
        let Some(bit) = 1u8.checked_shl(authority.index as u32) else {
            return false;
        };
        self.mapped_bars
            .get(slot)
            .is_some_and(|mapped| mapped & bit != 0)
    }
}

/// Add bus mastering through one live function-bound BAR mapping, isolated by
/// a DMA domain established for that same function.
///
/// # Errors
///
/// Refuses a domain for another function before any access, then a stale or
/// foreign BAR mapping, a command state from another function, or any
/// configuration access or exact-readback failure.
pub fn admit_mapped_bus_master<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &BarAuthority,
    isolated: Address,
    state: &mut CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if authority.function.at.function_base() != isolated.function_base() {
        return Err(EnableError::ForeignDomain);
    }
    if !functions.holds_bar(authority) {
        return Err(EnableError::Stale);
    }
    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)
}

/// Reserve one probed memory BAR under its exact function generation.
///
/// # Errors
///
/// Refuses a stale or cross-function probe, a non-memory slot, or a BAR whose
/// existing mapping authority has not been returned.
pub fn bind_memory_bar<const N: usize>(
    functions: &mut FunctionClaims<N>,
    function: &FunctionAuthority,
    bars: &ProbedBars,
    index: usize,
) -> Result<BarAuthority, BarAuthorityError> {
    if !functions.holds(function) || bars.function != Some(function.lease) {
        return Err(BarAuthorityError::Stale);
    }
    let Some(
        window @ Window {
            bar: Bar::Memory { .. },
            ..
        },
    ) = bars.window(index)
    else {
        return Err(BarAuthorityError::NotMemory);
    };
    let Some(slot) = functions.slot_of(function.lease) else {
        return Err(BarAuthorityError::Stale);
    };
    let Some(bit) = 1u8.checked_shl(index as u32) else {
        return Err(BarAuthorityError::NotMemory);
    };
    let Some(mapped) = functions.mapped_bars.get_mut(slot) else {
        return Err(BarAuthorityError::Stale);
    };
    if *mapped & bit != 0 {
        return Err(BarAuthorityError::Busy);
    }
    *mapped |= bit;
    Ok(BarAuthority {
        function: function.lease,
        index,
        window,
    })
}

/// Return one exact BAR mapping authority to its function registry.
///
/// # Errors
///
/// Returns the unchanged authority if its function generation or BAR slot is
/// no longer live in this registry.
pub fn release_memory_bar<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: BarAuthority,
) -> Result<(), BarAuthorityFailure> {
    let Some(slot) = functions.slot_of(authority.function) else {
        return Err(BarAuthorityFailure {
            reason: BarAuthorityError::Stale,
            authority,
        });
    };
    let Some(bit) = 1u8.checked_shl(authority.index as u32) else {
        return Err(BarAuthorityFailure {
            reason: BarAuthorityError::Stale,
            authority,
        });
    };
    let Some(mapped) = functions.mapped_bars.get_mut(slot) else {
        return Err(BarAuthorityFailure {
            reason: BarAuthorityError::Stale,
            authority,
        });
    };
    if *mapped & bit == 0 {
        return Err(BarAuthorityFailure {
            reason: BarAuthorityError::Stale,
            authority,
        });
    }
    *mapped &= !bit;
    Ok(())
}

/// Re-read a mapped BAR and require its exact decoded identity to be unchanged.
///
/// The closure performs only reads. A caller can therefore compose this check
/// and a command transaction under one hardware lock without any write before
/// identity validation succeeds.
///
/// # Errors
///
/// Refuses a stale or foreign mapping authority, an inaccessible BAR half, or
/// any decoded identity change.
fn verify_memory_bar<const N: usize>(
    functions: &FunctionClaims<N>,
    function: &FunctionAuthority,
    authority: &BarAuthority,
    mut read: impl FnMut(Address, usize) -> Option<u32>,
) -> Result<(), BarIdentityError> {
    if function.lease != authority.function
        || !functions.holds(function)
        || !functions.holds_bar(authority)
    {
        return Err(BarIdentityError::Stale);
    }
    let low_offset = u16::try_from(authority.index)
        .ok()
        .and_then(|index| index.checked_mul(4))
        .and_then(|offset| header::BAR0.checked_add(offset))
        .ok_or(BarIdentityError::Stale)?;
    let low_at = function
        .address()
        .at(low_offset)
        .ok_or(BarIdentityError::Stale)?;
    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)?;
    let high = match authority.window.bar {
        Bar::Memory { wide: true, .. } => {
            let high_offset = low_offset.checked_add(4).ok_or(BarIdentityError::Stale)?;
            if authority.index + 1 >= function.bar_registers() {
                return Err(BarIdentityError::Stale);
            }
            let high_at = function
                .address()
                .at(high_offset)
                .ok_or(BarIdentityError::Stale)?;
            read(high_at, authority.index + 1).ok_or(BarIdentityError::Access)?
        }
        Bar::Memory { wide: false, .. } => 0,
        Bar::Io { .. } | Bar::Absent => return Err(BarIdentityError::Stale),
    };
    if Bar::decode(low, high) != Some(authority.window.bar) {
        return Err(BarIdentityError::Changed);
    }
    Ok(())
}

/// Enable memory decode from a staged probe after its BAR identity is re-proven.
///
/// BAR reads and the command transition share one closure so a platform
/// adapter can hold its configuration lock across the complete sequence.
///
/// # Errors
///
/// Identity refusal performs no command access. Every command refusal leaves
/// the caller's restoration authority in `state`.
pub fn enable_memory_bar<const N: usize>(
    functions: &mut FunctionClaims<N>,
    function: &FunctionAuthority,
    authority: &BarAuthority,
    state: &mut CommandState,
    mut access: impl FnMut(Address, BarEnableAccess) -> Option<u32>,
) -> Result<(), BarEnableError> {
    verify_memory_bar(functions, function, authority, |at, index| {
        access(at, BarEnableAccess::ReadBar(index))
    })
    .map_err(BarEnableError::Identity)?;
    admit_memory_space_with_leases(
        &mut functions.commands,
        function.lease,
        state,
        |at, command| access(at, BarEnableAccess::Command(command)),
    )
    .map_err(BarEnableError::Command)
}

fn admit_memory_space_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    function: FunctionLease,
    state: &mut CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if state.phase != CommandPhase::Installed || !leases.holds_function(function, state.lease) {
        return Err(EnableError::Stale);
    }
    let at = state.lease.command_address();
    let current = match access(at, CommandAccess::Read) {
        Some(bits) => bits as u16,
        None => return Err(EnableError::Access),
    };
    if current != state.installed {
        return Err(EnableError::Changed(current));
    }
    let wanted = (current | command::MEMORY_SPACE) & !command::BUS_MASTER;
    state.phase = CommandPhase::Restoring(wanted);
    let result = change_command(at, current, wanted, &mut access);
    match result {
        Ok(()) => {
            state.installed = wanted;
            state.phase = CommandPhase::Installed;
            Ok(())
        }
        Err(EnableError::Restore) => Err(EnableError::Restore),
        Err(error) => {
            state.phase = CommandPhase::Installed;
            Err(error)
        }
    }
}

/// Why a BAR-probe transaction could not produce windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProbeError<E> {
    /// The function or command-transition reservation was unavailable.
    Unavailable(EnableError),
    /// The configuration mechanism refused an access before rollback failed.
    Access(E),
    /// The command register did not accept the exact decode-off word.
    Refused(u16),
    /// At least one original register could not be put back.
    Restore(E),
    /// A restoration readback did not equal the captured register word.
    RestoreChanged,
    /// The header layout, BAR slot use, or memory encoding is unsupported.
    Malformed,
    /// A device returned a mask or base that cannot describe its window.
    InvalidWindow,
}

impl<E: core::fmt::Display> core::fmt::Display for ProbeError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable(reason) => {
                write!(f, "bar probe authority is unavailable: {reason}")
            }
            Self::Access(reason) => write!(f, "configuration access failed: {reason}"),
            Self::Refused(observed) => write!(
                f,
                "command register refused decode-off value and read back {observed:#06x}"
            ),
            Self::Restore(reason) => write!(f, "configuration restoration failed: {reason}"),
            Self::RestoreChanged => core::fmt::Write::write_str(
                f,
                "configuration restoration changed the captured register state",
            ),
            Self::Malformed => {
                core::fmt::Write::write_str(f, "the function has an unsupported BAR layout")
            }
            Self::InvalidWindow => {
                core::fmt::Write::write_str(f, "a BAR returned an invalid memory window")
            }
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for ProbeError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Unavailable(reason) => Some(reason),
            Self::Access(reason) | Self::Restore(reason) => Some(reason),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeRestorePhase {
    Bars,
    Command,
}

#[derive(Debug, PartialEq, Eq)]
struct ProbeRestore {
    original: [u32; BARS],
    modified: u8,
    command: u16,
    phase: ProbeRestorePhase,
}

impl ProbeRestore {
    const fn modified(&self, index: usize) -> bool {
        self.modified & (1 << index) != 0
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RawProbeFailure<E> {
    reason: ProbeError<E>,
    restore: Option<ProbeRestore>,
}

#[derive(Debug, PartialEq, Eq)]
struct RawStagedProbe {
    bars: ProbedBars,
    restore: ProbeRestore,
}

/// A refused BAR probe, including exact retry authority when restoration is uncertain.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "an uncertain BAR probe may carry exclusive restoration authority"]
pub struct ProbeFailure<E> {
    reason: ProbeError<E>,
    retry: Option<ProbeRetry>,
}

impl<E> ProbeFailure<E> {
    /// Refuse before a probe reservation exists.
    pub fn unavailable(reason: EnableError) -> Self {
        Self {
            reason: ProbeError::Unavailable(reason),
            retry: None,
        }
    }

    /// Refuse while preserving an existing restoration owner.
    pub fn with_retry(reason: ProbeError<E>, retry: ProbeRetry) -> Self {
        Self {
            reason,
            retry: Some(retry),
        }
    }

    /// Why the probe refused.
    pub const fn reason(&self) -> &ProbeError<E> {
        &self.reason
    }

    /// Recover the same function reservation and captured register words.
    pub fn into_retry(self) -> Option<ProbeRetry> {
        self.retry
    }
}

/// The exclusive right to retry one uncertain BAR restoration.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "an uncertain BAR probe must be retried or retained fail-stopped"]
pub struct ProbeRetry {
    probe: CommandProbe,
    restore: ProbeRestore,
}

/// Probe every BAR under one exclusive decode-off transaction.
///
/// The closure is called for configuration cycles while the platform adapter
/// holds its configuration lock. I/O decode, memory decode and bus mastering
/// are cleared and read back before the first BAR write. BARs are restored
/// while that state remains verified; the successful result retains exclusive
/// authority to restore the original command later. A 64-bit BAR has both
/// halves filled with ones before either half is read.
///
/// # Errors
///
/// Refuses invalid layout, size, alignment, or configuration access. A clean
/// failure restores every modified register and the original command;
/// uncertain restoration keeps DMA and decode disabled and returns retry
/// authority in [`ProbeFailure::into_retry`].
pub fn probe_bars<const N: usize, E>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, ProbeAccess) -> Result<u32, E>,
) -> Result<StagedProbe, ProbeFailure<E>> {
    if !functions.holds(authority) {
        return Err(ProbeFailure::unavailable(EnableError::Stale));
    }
    probe_bars_with_leases(&mut functions.commands, authority, access)
}

fn probe_bars_with_leases<const N: usize, E>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    mut access: impl FnMut(Address, ProbeAccess) -> Result<u32, E>,
) -> Result<StagedProbe, ProbeFailure<E>> {
    let probe = leases
        .begin_probe(authority)
        .map_err(|reason| ProbeFailure {
            reason: ProbeError::Unavailable(reason),
            retry: None,
        })?;
    let function = probe.0.function.at;
    match probe_bars_unleased(authority.bar_registers(), |event| access(function, event)) {
        Ok(staged) => {
            let RawStagedProbe { mut bars, restore } = staged;
            let command = match leases.stage_probe(authority, probe, restore.command) {
                Ok(command) => command,
                Err(probe) => {
                    return Err(ProbeFailure {
                        reason: ProbeError::Unavailable(EnableError::Stale),
                        retry: Some(ProbeRetry { probe, restore }),
                    });
                }
            };
            bars.function = Some(authority.lease);
            Ok(StagedProbe { bars, command })
        }
        Err(failure) => match failure.restore {
            Some(restore) => Err(ProbeFailure {
                reason: failure.reason,
                retry: Some(ProbeRetry { probe, restore }),
            }),
            None => {
                let reason = failure.reason;
                assert!(
                    leases.finish_probe(authority, probe).is_ok(),
                    "a restored BAR-probe refusal lost its exact reservation"
                );
                Err(ProbeFailure {
                    reason,
                    retry: None,
                })
            }
        },
    }
}

/// Retry a BAR restoration through the same function reservation.
///
/// # Errors
///
/// Returns the unchanged retry authority when access or exact readback still
/// cannot prove the captured device state.
pub fn retry_probe<const N: usize, E>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    retry: ProbeRetry,
    access: impl FnMut(Address, ProbeAccess) -> Result<u32, E>,
) -> Result<(), ProbeFailure<E>> {
    if !functions.holds(authority) {
        return Err(ProbeFailure::with_retry(
            ProbeError::Unavailable(EnableError::Stale),
            retry,
        ));
    }
    retry_probe_with_leases(&mut functions.commands, authority, retry, access)
}

fn retry_probe_with_leases<const N: usize, E>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    retry: ProbeRetry,
    mut access: impl FnMut(Address, ProbeAccess) -> Result<u32, E>,
) -> Result<(), ProbeFailure<E>> {
    if !leases.holds(authority, retry.probe.0) {
        return Err(ProbeFailure {
            reason: ProbeError::Unavailable(EnableError::Stale),
            retry: Some(retry),
        });
    }
    let function = retry.probe.0.function.at;
    match finish_probe_restoration(retry.restore, &mut |event| access(function, event)) {
        Ok(()) => {
            assert!(
                leases.finish_probe(authority, retry.probe).is_ok(),
                "a restored BAR retry lost its exact reservation"
            );
            Ok(())
        }
        Err((reason, restore)) => Err(ProbeFailure {
            reason,
            retry: Some(ProbeRetry {
                probe: retry.probe,
                restore,
            }),
        }),
    }
}

fn probe_bars_unleased<E>(
    registers: usize,
    mut access: impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<RawStagedProbe, RawProbeFailure<E>> {
    if registers > BARS {
        return Err(RawProbeFailure {
            reason: ProbeError::Malformed,
            restore: None,
        });
    }

    let command = access(ProbeAccess::ReadCommand).map_err(|error| RawProbeFailure {
        reason: ProbeError::Access(error),
        restore: None,
    })? as u16;
    let mut original = [0u32; BARS];
    for (index, slot) in original.iter_mut().enumerate().take(registers) {
        *slot = access(ProbeAccess::ReadBar(index)).map_err(|error| RawProbeFailure {
            reason: ProbeError::Access(error),
            restore: None,
        })?;
    }

    let Some(plan) = bar_slots(registers, &original) else {
        return Err(RawProbeFailure {
            reason: ProbeError::Malformed,
            restore: None,
        });
    };

    let mut modified = 0u8;
    let decode_off = command & !PROBE_DISABLED;
    let command_restore = ProbeRestore {
        original,
        modified,
        command,
        phase: ProbeRestorePhase::Command,
    };
    if let Err(error) = access(ProbeAccess::WriteCommand(decode_off)) {
        return Err(after_access_error(error, &mut access, command_restore));
    }
    match access(ProbeAccess::ReadCommand) {
        Ok(bits) if bits as u16 == decode_off => {}
        Ok(bits) => {
            return Err(after_probe_error(
                ProbeError::Refused(bits as u16),
                &mut access,
                command_restore,
            ));
        }
        Err(error) => {
            return Err(after_access_error(error, &mut access, command_restore));
        }
    }

    let mut masks = [0u32; BARS];
    for slots in plan.iter().flatten() {
        let index = slots.low;
        modified |= 1 << index;
        if let Err(error) = access(ProbeAccess::WriteBar(index, u32::MAX)) {
            return Err(after_access_error(
                error,
                &mut access,
                ProbeRestore {
                    original,
                    modified,
                    command,
                    phase: ProbeRestorePhase::Bars,
                },
            ));
        }
        if let Some(high) = slots.high {
            modified |= 1 << high;
            if let Err(error) = access(ProbeAccess::WriteBar(high, u32::MAX)) {
                return Err(after_access_error(
                    error,
                    &mut access,
                    ProbeRestore {
                        original,
                        modified,
                        command,
                        phase: ProbeRestorePhase::Bars,
                    },
                ));
            }
        }

        masks[index] = match access(ProbeAccess::ReadBar(index)) {
            Ok(mask) => mask,
            Err(error) => {
                return Err(after_access_error(
                    error,
                    &mut access,
                    ProbeRestore {
                        original,
                        modified,
                        command,
                        phase: ProbeRestorePhase::Bars,
                    },
                ));
            }
        };
        if let Some(high) = slots.high {
            masks[high] = match access(ProbeAccess::ReadBar(high)) {
                Ok(mask) => mask,
                Err(error) => {
                    return Err(after_access_error(
                        error,
                        &mut access,
                        ProbeRestore {
                            original,
                            modified,
                            command,
                            phase: ProbeRestorePhase::Bars,
                        },
                    ));
                }
            };
        }
    }

    let restore = ProbeRestore {
        original,
        modified,
        command,
        phase: ProbeRestorePhase::Bars,
    };
    let restore = match restore_probe_bars(restore, &mut access) {
        Ok(restore) => restore,
        Err((reason, restore)) => {
            return Err(RawProbeFailure {
                reason,
                restore: Some(restore),
            });
        }
    };
    let windows = match decode_windows(&plan, &original, &masks) {
        Ok(windows) => windows,
        Err(reason) => return Err(after_probe_error(reason, &mut access, restore)),
    };
    Ok(RawStagedProbe {
        bars: ProbedBars {
            function: None,
            windows,
        },
        restore,
    })
}

fn restore_probe_bars<E>(
    mut restore: ProbeRestore,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<ProbeRestore, (ProbeError<E>, ProbeRestore)> {
    if restore.phase == ProbeRestorePhase::Bars {
        let decode_off = restore.command & !PROBE_DISABLED;
        let _ = access(ProbeAccess::WriteCommand(decode_off));
        match access(ProbeAccess::ReadCommand) {
            Ok(bits) if bits as u16 == decode_off => {}
            Ok(_) => return Err((ProbeError::RestoreChanged, restore)),
            Err(error) => return Err((ProbeError::Restore(error), restore)),
        }

        for index in (0..BARS).rev() {
            if restore.modified(index) {
                let _ = access(ProbeAccess::WriteBar(index, restore.original[index]));
            }
        }
        for index in (0..BARS).rev() {
            if !restore.modified(index) {
                continue;
            }
            match access(ProbeAccess::ReadBar(index)) {
                Ok(bits) if bits == restore.original[index] => {}
                Ok(_) => return Err((ProbeError::RestoreChanged, restore)),
                Err(error) => return Err((ProbeError::Restore(error), restore)),
            }
        }
        restore.phase = ProbeRestorePhase::Command;
    }
    Ok(restore)
}

fn finish_probe_restoration<E>(
    restore: ProbeRestore,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<(), (ProbeError<E>, ProbeRestore)> {
    let restore = restore_probe_bars(restore, access)?;
    let _ = access(ProbeAccess::WriteCommand(restore.command));
    match access(ProbeAccess::ReadCommand) {
        Ok(bits) if bits as u16 == restore.command => Ok(()),
        Ok(_) => Err((ProbeError::RestoreChanged, restore)),
        Err(error) => Err((ProbeError::Restore(error), restore)),
    }
}

#[cfg(test)]
fn restore_staged_probe<E>(
    staged: RawStagedProbe,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<ProbedBars, RawProbeFailure<E>> {
    if let Err((reason, restore)) = finish_probe_restoration(staged.restore, access) {
        return Err(RawProbeFailure {
            reason,
            restore: Some(restore),
        });
    }
    Ok(staged.bars)
}

fn bar_slots(registers: usize, original: &[u32; BARS]) -> Option<[Option<BarSlots>; BARS]> {
    let mut plan = [None; BARS];
    let mut indices = 0..registers;
    while let Some(low) = core::iter::Iterator::next(&mut indices) {
        let high = if matches!(
            Bar::decode(original[low], 0)?,
            Bar::Memory { wide: true, .. }
        ) {
            Some(core::iter::Iterator::next(&mut indices)?)
        } else {
            None
        };
        plan[low] = Some(BarSlots { low, high });
    }
    Some(plan)
}

fn after_access_error<E>(
    error: E,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    restore: ProbeRestore,
) -> RawProbeFailure<E> {
    after_probe_error(ProbeError::Access(error), access, restore)
}

fn after_probe_error<E>(
    reason: ProbeError<E>,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    restore: ProbeRestore,
) -> RawProbeFailure<E> {
    match finish_probe_restoration(restore, access) {
        Ok(()) => RawProbeFailure {
            reason,
            restore: None,
        },
        Err((reason, restore)) => RawProbeFailure {
            reason,
            restore: Some(restore),
        },
    }
}

fn decode_windows<E>(
    plan: &[Option<BarSlots>; BARS],
    original: &[u32; BARS],
    masks: &[u32; BARS],
) -> Result<[Option<Window>; BARS], ProbeError<E>> {
    let mut windows = [None; BARS];
    for slots in plan.iter().flatten() {
        let index = slots.low;
        let high = slots.high.map_or(0, |high| original[high]);
        let mask_high = slots.high.map_or(0, |high| masks[high]);
        let (bar, size) = match Bar::decode(original[index], high).ok_or(ProbeError::Malformed)? {
            Bar::Io { port } => {
                let Some(size) = io_window_size(masks[index]) else {
                    continue;
                };
                (Bar::Io { port }, u64::from(size))
            }
            bar @ Bar::Memory { wide, .. } => {
                let width = if wide {
                    MemoryWidth::Bits64
                } else {
                    MemoryWidth::Bits32
                };
                let Some(size) = memory_window_size(masks[index], mask_high, width) else {
                    continue;
                };
                (bar, size)
            }
            Bar::Absent => {
                let Some(size) = memory_window_size(masks[index], 0, MemoryWidth::Bits32) else {
                    continue;
                };
                (
                    Bar::Memory {
                        address: 0,
                        wide: false,
                        prefetchable: false,
                    },
                    size,
                )
            }
        };

        let base = match bar {
            Bar::Memory { address, .. } => address,
            Bar::Io { port } => u64::from(port),
            Bar::Absent => 0,
        };
        if !size.is_power_of_two() || !base.is_multiple_of(size) {
            return Err(ProbeError::InvalidWindow);
        }
        windows[index] = Some(Window { bar, size });
    }
    Ok(windows)
}

/// Whether a vendor word means "nothing is here".
///
/// An absent function does not fail a configuration read; the bus returns all
/// ones, so `0xFFFF` is the answer and not an error. A driver that treats it
/// as a vendor id enumerates 256 buses of imaginary devices.
pub const fn is_absent(vendor: u16) -> bool {
    vendor == 0xFFFF
}

/// Buses in one legacy PCI segment.
const BUSES: usize = 256;

/// Function identities in one legacy PCI segment.
const SEGMENT_FUNCTIONS: usize = BUSES * DEVICES as usize * FUNCTIONS as usize;

/// Machine words needed to remember every function identity in one segment.
const PRESENCE_WORDS: usize = SEGMENT_FUNCTIONS.div_ceil(u64::BITS as usize);

/// A bridge range admitted into a sealed [`PciTopology`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeRange {
    /// Bus immediately behind the bridge.
    pub secondary: u8,
    /// Highest bus the bridge forwards to.
    pub subordinate: u8,
}

/// Why a complete PCI topology snapshot cannot be trusted for DMA authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TopologyError {
    /// The function named by entry-fence authority was absent from the later snapshot.
    QualifiedNonRequesterMissing {
        /// Exact function which had been classified at entry.
        at: Address,
    },
    /// The function named by entry-fence authority changed before the snapshot.
    QualifiedNonRequesterChanged {
        /// Exact function whose identity, class or command changed.
        at: Address,
    },
    /// The requester quiesced at entry was absent from the later snapshot.
    QuiescedRequesterMissing {
        /// Exact function which had been quiesced at entry.
        at: Address,
    },
    /// The requester quiesced at entry changed before the snapshot.
    QuiescedRequesterChanged {
        /// Exact function whose identity, class or command changed.
        at: Address,
    },
    /// The class and header layout disagree about whether a function is a bridge.
    BridgeIdentity {
        /// Function with contradictory bridge identity.
        at: Address,
    },
    /// A bridge function had no readable bus-number register.
    MissingBusNumbers {
        /// Bridge whose routing could not be read.
        at: Address,
    },
    /// A bridge's primary or forwarded range cannot describe a child bus.
    InvalidRange {
        /// Bridge with invalid bus numbers.
        at: Address,
        /// Primary bus encoded by the bridge.
        primary: u8,
        /// First forwarded bus.
        secondary: u8,
        /// Last forwarded bus.
        subordinate: u8,
    },
    /// Two different bridges claim to be one bus's direct parent.
    DuplicateParent {
        /// Bus with two parent bridges.
        bus: u8,
        /// First parent found.
        first: Address,
        /// Second parent found.
        second: Address,
    },
    /// A present function is on a bus with no rooted parent chain.
    UnrootedFunction {
        /// Present function outside the proved tree.
        at: Address,
    },
    /// A child bridge forwards beyond the range admitted by its parent.
    ChildEscapesParent {
        /// Child bridge whose range escapes.
        child: Address,
        /// Direct parent bridge.
        parent: Address,
    },
    /// Two unrelated bridge branches advertise intersecting ranges.
    UnrelatedOverlap {
        /// First bridge in the overlap.
        first: Address,
        /// Second bridge in the overlap.
        second: Address,
    },
}

impl core::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::QualifiedNonRequesterMissing { at } => write!(
                f,
                "entry-qualified PCI non-requester {:02x}:{:02x}.{} is absent from the sealed snapshot",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::QualifiedNonRequesterChanged { at } => write!(
                f,
                "entry-qualified PCI non-requester {:02x}:{:02x}.{} changed before the sealed snapshot",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::QuiescedRequesterMissing { at } => write!(
                f,
                "entry-quiesced PCI requester {:02x}:{:02x}.{} is absent from the sealed snapshot",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::QuiescedRequesterChanged { at } => write!(
                f,
                "entry-quiesced PCI requester {:02x}:{:02x}.{} changed before the sealed snapshot",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::BridgeIdentity { at } => write!(
                f,
                "PCI function {:02x}:{:02x}.{} has contradictory bridge class and header",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::MissingBusNumbers { at } => write!(
                f,
                "PCI bridge {:02x}:{:02x}.{} has no readable bus numbers",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::InvalidRange {
                at,
                primary,
                secondary,
                subordinate,
            } => write!(
                f,
                "PCI bridge {:02x}:{:02x}.{} reports invalid buses {primary:#04x}/{secondary:#04x}/{subordinate:#04x}",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::DuplicateParent { bus, first, second } => write!(
                f,
                "PCI bus {bus:#04x} has parent bridges {:02x}:{:02x}.{} and {:02x}:{:02x}.{}",
                first.bus(),
                first.device(),
                first.function(),
                second.bus(),
                second.device(),
                second.function()
            ),
            Self::UnrootedFunction { at } => write!(
                f,
                "PCI function {:02x}:{:02x}.{} has no unique path to bus zero",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::ChildEscapesParent { child, parent } => write!(
                f,
                "PCI bridge {:02x}:{:02x}.{} escapes parent {:02x}:{:02x}.{}",
                child.bus(),
                child.device(),
                child.function(),
                parent.bus(),
                parent.device(),
                parent.function()
            ),
            Self::UnrelatedOverlap { first, second } => write!(
                f,
                "unrelated PCI bridges {:02x}:{:02x}.{} and {:02x}:{:02x}.{} advertise overlapping buses",
                first.bus(),
                first.device(),
                first.function(),
                second.bus(),
                second.device(),
                second.function()
            ),
        }
    }
}

impl core::error::Error for TopologyError {}

/// A fixed, allocation-free snapshot of one rooted legacy PCI segment.
///
/// Each non-root bus is indexed by its unique direct parent bridge. Queries
/// return nothing until [`Self::seal`] proves that all present functions reach
/// bus zero, child ranges stay inside their parents, and unrelated branches
/// do not overlap.
#[derive(Debug, PartialEq, Eq)]
pub struct PciTopology {
    parent: [u32; BUSES],
    subordinate: [u8; BUSES],
    present: [u64; PRESENCE_WORDS],
    requesters: [u64; PRESENCE_WORDS],
    quiesced: Option<QuiescedRequester>,
    sealed: bool,
}

impl PciTopology {
    /// Begin an empty, unsealed segment-zero snapshot.
    pub const fn new() -> Self {
        Self {
            parent: [0; BUSES],
            subordinate: [0; BUSES],
            present: [0; PRESENCE_WORDS],
            requesters: [0; PRESENCE_WORDS],
            quiesced: None,
            sealed: false,
        }
    }

    /// Discard every observation and begin another snapshot.
    pub fn reset(&mut self) {
        self.parent.fill(0);
        self.subordinate.fill(0);
        self.present.fill(0);
        self.requesters.fill(0);
        self.quiesced = None;
        self.sealed = false;
    }

    /// Record one independently probed function and, for a bridge, its routing.
    ///
    /// # Errors
    ///
    /// Returns [`TopologyError`] for contradictory bridge identity, unreadable
    /// bus numbers, an invalid range, or a second direct parent for one bus.
    pub fn observe(
        &mut self,
        at: Address,
        class: Class,
        header: HeaderType,
        bus_numbers: Option<u32>,
    ) -> Result<(), TopologyError> {
        self.sealed = false;
        let at = at.function_base();
        let index = function_index(at);
        self.present[index / u64::BITS as usize] |= 1 << (index % u64::BITS as usize);
        self.requesters[index / u64::BITS as usize] |= 1 << (index % u64::BITS as usize);

        if class.is_bridge() != header.is_bridge() {
            return Err(TopologyError::BridgeIdentity { at });
        }
        if !class.is_bridge() {
            return Ok(());
        }
        let Some(bits) = bus_numbers else {
            return Err(TopologyError::MissingBusNumbers { at });
        };
        let primary = primary_bus(bits);
        let secondary = secondary_bus(bits);
        let subordinate = subordinate_bus(bits);
        if primary != at.bus() || secondary <= at.bus() || secondary > subordinate {
            return Err(TopologyError::InvalidRange {
                at,
                primary,
                secondary,
                subordinate,
            });
        }
        let slot = &mut self.parent[usize::from(secondary)];
        if *slot != 0 && *slot != at.bits() {
            return Err(TopologyError::DuplicateParent {
                bus: secondary,
                first: Address(*slot),
                second: at,
            });
        }
        *slot = at.bits();
        self.subordinate[usize::from(secondary)] = subordinate;
        Ok(())
    }

    /// Record an exact platform-qualified function that is present but is not
    /// a PCI DMA requester.
    ///
    /// # Errors
    ///
    /// The same topology errors as [`Self::observe`]. The consumed authority
    /// can name only the exact platform tuple recognized by the handoff policy.
    pub fn observe_non_requester(
        &mut self,
        authority: QualifiedNonRequester,
        observation: FunctionObservation,
    ) -> Result<(), TopologyError> {
        let FunctionObservation {
            at,
            identity,
            class_bits,
            command,
            class,
            header,
            bus_numbers,
        } = observation;
        let at = at.function_base();
        if at != authority.at
            || identity != authority.identity
            || class_bits != authority.class
            || command != authority.command
        {
            return Err(TopologyError::QualifiedNonRequesterChanged { at });
        }
        self.observe(at, class, header, bus_numbers)?;
        let index = function_index(at);
        self.requesters[index / u64::BITS as usize] &= !(1 << (index % u64::BITS as usize));
        Ok(())
    }

    /// Record the exact entry-quiesced ISA requester, which stays a requester.
    ///
    /// # Errors
    ///
    /// The same topology errors as [`Self::observe`], or
    /// [`TopologyError::QuiescedRequesterChanged`] when any field the entry
    /// fence bound differs.
    pub fn observe_quiesced_requester(
        &mut self,
        authority: QuiescedIsaRequester,
        observation: FunctionObservation,
    ) -> Result<(), TopologyError> {
        let at = observation.at.function_base();
        let observed = (
            at,
            observation.identity,
            observation.class_bits,
            observation.command,
        );
        let bound = (
            authority.at,
            authority.identity,
            authority.class,
            authority.command,
        );
        if observed != bound {
            return Err(TopologyError::QuiescedRequesterChanged { at });
        }
        self.observe(
            at,
            observation.class,
            observation.header,
            observation.bus_numbers,
        )?;
        self.quiesced = Some(QuiescedRequester {
            at,
            identity: authority.identity,
            command: authority.command,
        });
        Ok(())
    }

    /// The entry-quiesced requester at `at`, once the snapshot is sealed.
    pub fn quiesced_requester(&self, at: Address) -> Option<QuiescedRequester> {
        if !self.sealed {
            return None;
        }
        self.quiesced
            .filter(|record| record.at == at.function_base())
    }

    /// Seal the snapshot after validating its complete rooted interval tree.
    ///
    /// # Errors
    ///
    /// Returns [`TopologyError`] when a present function is unrooted, a child
    /// range escapes its parent, or unrelated branches overlap.
    pub fn seal(&mut self) -> Result<(), TopologyError> {
        self.sealed = false;
        for index in 0..SEGMENT_FUNCTIONS {
            if self.present[index / u64::BITS as usize] & (1 << (index % u64::BITS as usize)) == 0 {
                continue;
            }
            let at = address_for_index(index);
            if at.bus() != 0 && !self.bus_reaches_root(at.bus()) {
                return Err(TopologyError::UnrootedFunction { at });
            }
        }

        for secondary in 1u16..=u16::from(u8::MAX) {
            let secondary = secondary as u8;
            let Some(child) = self.parent(secondary) else {
                continue;
            };
            if child.bus() == 0 {
                continue;
            }
            let Some(parent) = self.parent(child.bus()) else {
                return Err(TopologyError::UnrootedFunction { at: child });
            };
            let parent_secondary = self.secondary_for(parent);
            if secondary < parent_secondary
                || self.subordinate[usize::from(secondary)]
                    > self.subordinate[usize::from(parent_secondary)]
            {
                return Err(TopologyError::ChildEscapesParent { child, parent });
            }
        }

        for first_bus in 1u16..=u16::from(u8::MAX) {
            let first_bus = first_bus as u8;
            let Some(first) = self.parent(first_bus) else {
                continue;
            };
            for second_bus in (u16::from(first_bus) + 1)..=u16::from(u8::MAX) {
                let second_bus = second_bus as u8;
                let Some(second) = self.parent(second_bus) else {
                    continue;
                };
                let overlap = first_bus <= self.subordinate[usize::from(second_bus)]
                    && second_bus <= self.subordinate[usize::from(first_bus)];
                if overlap
                    && !self.bridge_is_ancestor(first, second)
                    && !self.bridge_is_ancestor(second, first)
                {
                    return Err(TopologyError::UnrelatedOverlap { first, second });
                }
            }
        }
        self.sealed = true;
        Ok(())
    }

    /// Whether the complete snapshot passed validation.
    pub const fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Whether one exact function was present in the sealed snapshot.
    pub fn contains_function(&self, at: Address) -> bool {
        if !self.sealed {
            return false;
        }
        let index = function_index(at.function_base());
        self.present[index / u64::BITS as usize] & (1 << (index % u64::BITS as usize)) != 0
    }

    /// Whether one exact function is a DMA requester in the sealed snapshot.
    pub fn contains_requester(&self, at: Address) -> bool {
        if !self.sealed {
            return false;
        }
        let index = function_index(at.function_base());
        self.requesters[index / u64::BITS as usize] & (1 << (index % u64::BITS as usize)) != 0
    }

    /// The sealed routing range of one exact bridge function.
    pub fn bridge_range(&self, at: Address) -> Option<BridgeRange> {
        if !self.sealed {
            return None;
        }
        let at = at.function_base();
        for secondary in 1u16..=u16::from(u8::MAX) {
            let secondary = secondary as u8;
            if self.parent(secondary) == Some(at) {
                return Some(BridgeRange {
                    secondary,
                    subordinate: self.subordinate[usize::from(secondary)],
                });
            }
        }
        None
    }

    /// Whether `target` is `root` or lies below that bridge in the sealed tree.
    pub fn hierarchy_contains(&self, root: Address, target: Address) -> bool {
        if !self.sealed {
            return false;
        }
        let root = root.function_base();
        let target = target.function_base();
        if root == target {
            return self.contains_function(root);
        }
        self.bridge_range(root).is_some()
            && self.contains_function(target)
            && self.bridge_is_ancestor(root, target)
    }

    fn parent(&self, bus: u8) -> Option<Address> {
        let bits = self.parent[usize::from(bus)];
        (bits != 0).then_some(Address(bits))
    }

    fn secondary_for(&self, bridge: Address) -> u8 {
        for secondary in 1u16..=u16::from(u8::MAX) {
            let secondary = secondary as u8;
            if self.parent(secondary) == Some(bridge) {
                return secondary;
            }
        }
        0
    }

    fn bus_reaches_root(&self, mut bus: u8) -> bool {
        for _ in 0..BUSES_BELOW_ROOT {
            let Some(parent) = self.parent(bus) else {
                return false;
            };
            bus = parent.bus();
            if bus == 0 {
                return true;
            }
        }
        false
    }

    fn bridge_is_ancestor(&self, ancestor: Address, descendant: Address) -> bool {
        let ancestor = ancestor.function_base();
        let mut bus = descendant.bus();
        for _ in 0..BUSES_BELOW_ROOT {
            let Some(parent) = self.parent(bus) else {
                return false;
            };
            if parent == ancestor {
                return true;
            }
            bus = parent.bus();
            if bus == 0 {
                return false;
            }
        }
        false
    }
}

impl Default for PciTopology {
    fn default() -> Self {
        Self::new()
    }
}

/// Non-root buses in one segment; parent walks must strictly approach bus zero.
const BUSES_BELOW_ROOT: usize = BUSES - 1;

fn function_index(at: Address) -> usize {
    (usize::from(at.bus()) * usize::from(DEVICES) + usize::from(at.device()))
        * usize::from(FUNCTIONS)
        + usize::from(at.function())
}

fn address_for_index(index: usize) -> Address {
    let bus = index / (usize::from(DEVICES) * usize::from(FUNCTIONS));
    let on_bus = index % (usize::from(DEVICES) * usize::from(FUNCTIONS));
    let device = on_bus / usize::from(FUNCTIONS);
    let function = on_bus % usize::from(FUNCTIONS);
    Address(
        Address::ENABLE | ((bus as u32) << 16) | ((device as u32) << 11) | ((function as u32) << 8),
    )
}

/// A bridge's primary bus number, out of the word at [`header::BUS_NUMBERS`].
pub const fn primary_bus(bits: u32) -> u8 {
    bits as u8
}

/// A bridge's secondary bus number, out of the word at [`header::BUS_NUMBERS`].
///
/// The bus behind it, which is the number a recursive enumeration descends to.
pub const fn secondary_bus(bits: u32) -> u8 {
    (bits >> 8) as u8
}

/// A bridge's subordinate bus number, out of the word at [`header::BUS_NUMBERS`].
///
/// The highest bus behind it: the bridge forwards to every bus from its
/// secondary through this one.
pub const fn subordinate_bus(bits: u32) -> u8 {
    (bits >> 16) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TestAccessError;

    impl core::fmt::Display for TestAccessError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            core::fmt::Write::write_str(f, "test access failure")
        }
    }

    impl core::error::Error for TestAccessError {}

    #[test]
    fn every_probe_error_describes_and_exposes_its_cause() {
        fn assert_error<T: core::error::Error>() {}

        assert_error::<ProbeError<TestAccessError>>();
        for (error, expected) in [
            (
                ProbeError::Unavailable(EnableError::Stale),
                "bar probe authority is unavailable: the command lease is no longer current",
            ),
            (
                ProbeError::Access(TestAccessError),
                "configuration access failed: test access failure",
            ),
            (
                ProbeError::Refused(0x00a5),
                "command register refused decode-off value and read back 0x00a5",
            ),
            (
                ProbeError::Restore(TestAccessError),
                "configuration restoration failed: test access failure",
            ),
            (
                ProbeError::RestoreChanged,
                "configuration restoration changed the captured register state",
            ),
            (
                ProbeError::Malformed,
                "the function has an unsupported BAR layout",
            ),
            (
                ProbeError::InvalidWindow,
                "a BAR returned an invalid memory window",
            ),
        ] {
            assert_eq!(error.to_string(), expected);
        }

        assert!(
            core::error::Error::source(&ProbeError::<TestAccessError>::Unavailable(
                EnableError::Stale
            ))
            .is_some()
        );
        assert!(core::error::Error::source(&ProbeError::Access(TestAccessError)).is_some());
        assert!(core::error::Error::source(&ProbeError::Restore(TestAccessError)).is_some());
        assert!(core::error::Error::source(&ProbeError::<TestAccessError>::Malformed).is_none());
    }

    #[test]
    fn the_handoff_fence_exhausts_the_segment_and_preserves_command_bits() {
        let first = Address::new(0, 1, 0, 0).expect("first function");
        let last = Address::new(u8::MAX, DEVICES - 1, FUNCTIONS - 1, 0).expect("last function");
        let first_identity = 0x1234_5678;
        let last_identity = 0x89ab_cdef;
        let mut first_command =
            command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER | (1 << 10);
        let mut last_command = command::MEMORY_SPACE | (1 << 8);

        let report = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
            let function = at.function_base();
            let identity = if function == first {
                first_identity
            } else if function == last {
                last_identity
            } else {
                u32::MAX
            };
            match operation {
                HandoffAccess::ReadIdentity => Some(identity),
                HandoffAccess::ReadClass => Some(0),
                HandoffAccess::ReadCommand if function == first => Some(u32::from(first_command)),
                HandoffAccess::ReadCommand if function == last => Some(u32::from(last_command)),
                HandoffAccess::WriteCommand(value) if function == first => {
                    first_command = value;
                    Some(0)
                }
                HandoffAccess::WriteCommand(value) if function == last => {
                    last_command = value;
                    Some(0)
                }
                _ => None,
            }
        })
        .expect("the complete static segment is fenced");

        assert_eq!(report.functions(), 2);
        assert_eq!(report.disabled(), 1);
        assert_eq!(report.retained_non_requesters(), 0);
        assert_eq!(
            first_command,
            command::IO_SPACE | command::MEMORY_SPACE | (1 << 10)
        );
        assert_eq!(last_command, command::MEMORY_SPACE | (1 << 8));
    }

    #[test]
    fn every_handoff_fence_uncertainty_refuses_the_boundary() {
        let target = Address::new(0, 0, 0, 0).expect("target function");
        let identity = 0x1234_5678;

        let access = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
            if at.function_base() != target {
                return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
            }
            match operation {
                HandoffAccess::ReadIdentity => Some(identity),
                HandoffAccess::ReadClass => Some(0),
                HandoffAccess::ReadCommand => None,
                HandoffAccess::WriteCommand(_) => Some(0),
                HandoffAccess::QuiesceIsaDma => None,
            }
        });
        assert!(matches!(
            access,
            Err(HandoffFenceError::Access {
                operation: HandoffAccess::ReadCommand,
                ..
            })
        ));

        let refused = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
            if at.function_base() != target {
                return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
            }
            match operation {
                HandoffAccess::ReadIdentity => Some(identity),
                HandoffAccess::ReadClass => Some(0),
                HandoffAccess::ReadCommand => Some(u32::from(command::BUS_MASTER)),
                HandoffAccess::WriteCommand(_) => Some(0),
                HandoffAccess::QuiesceIsaDma => None,
            }
        });
        assert!(matches!(
            refused,
            Err(HandoffFenceError::CommandRefused {
                at,
                identity: found_identity,
                class: 0,
                expected: 0,
                found: command::BUS_MASTER,
            }) if at == target && found_identity == identity
        ));

        let mut identity_reads = 0;
        let changed = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
            if at.function_base() != target {
                return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
            }
            match operation {
                HandoffAccess::ReadIdentity => {
                    identity_reads += 1;
                    Some(if identity_reads == 1 {
                        identity
                    } else {
                        identity ^ 1
                    })
                }
                HandoffAccess::ReadClass => Some(0),
                HandoffAccess::ReadCommand => Some(0),
                HandoffAccess::WriteCommand(_) => Some(0),
                HandoffAccess::QuiesceIsaDma => None,
            }
        });
        assert!(matches!(
            changed,
            Err(HandoffFenceError::IdentityChanged {
                at,
                expected,
                found,
            }) if at == target && expected == identity && found == identity ^ 1
        ));
    }

    /// VMware Workstation 17.6's default segment as its entry fence saw it on
    /// 2026-10-20: BDF, identity, class dword, command before, command after
    /// a BME-clear write. `00:00.0` and `00:07.0` hard-wire bit 2.
    type CapturedFunction = ((u8, u8, u8), u32, u32, u16, u16);

    const VMWARE_SEGMENT: &[CapturedFunction] = &[
        ((0x00, 0x00, 0), 0x7190_8086, 0x0600_0001, 0x0006, 0x0006),
        ((0x00, 0x01, 0), 0x7191_8086, 0x0604_0001, 0x0017, 0x0013),
        ((0x00, 0x07, 0), 0x7110_8086, 0x0601_0008, 0x0007, 0x0007),
        ((0x00, 0x07, 1), 0x7111_8086, 0x0101_8a01, 0x0005, 0x0001),
        ((0x00, 0x07, 3), 0x7113_8086, 0x0680_0008, 0x0000, 0x0000),
        ((0x00, 0x0f, 0), 0x0405_15ad, 0x0300_0000, 0x0007, 0x0003),
        ((0x00, 0x11, 0), 0x0790_15ad, 0x0604_0102, 0x0007, 0x0003),
        ((0x02, 0x00, 0), 0x0774_15ad, 0x0c03_0000, 0x0005, 0x0001),
        ((0x02, 0x01, 0), 0x07e0_15ad, 0x0106_0100, 0x0006, 0x0002),
        ((0x03, 0x00, 0), 0x07f0_15ad, 0x0108_0200, 0x0007, 0x0003),
        ((0x0b, 0x00, 0), 0x07f0_15ad, 0x0108_0200, 0x0007, 0x0003),
    ];

    /// The thirty-two VMware PCIe root ports at `00:15.0` through `00:18.7`.
    const VMWARE_ROOT_PORT: (u32, u32, u16, u16) = (0x07a0_15ad, 0x0604_0001, 0x0007, 0x0003);

    fn vmware_function(at: Address) -> Option<(u32, u32, u16, bool)> {
        let (bus, device, function) = (at.bus(), at.device(), at.function());
        if bus == 0 && (0x15..=0x18).contains(&device) {
            let (identity, class, before, _) = VMWARE_ROOT_PORT;
            return Some((identity, class, before, false));
        }
        VMWARE_SEGMENT
            .iter()
            .find(|(bdf, ..)| *bdf == (bus, device, function))
            .map(|&(_, identity, class, before, found)| {
                (
                    identity,
                    class,
                    before,
                    found == before && before & command::BUS_MASTER != 0,
                )
            })
    }

    #[test]
    fn the_captured_vmware_segment_fences_and_fits_the_function_authority_limit() {
        let mut commands = [[[None::<u16>; FUNCTIONS as usize]; DEVICES as usize]; 256];
        let mut quiesces = 0;
        let mut report = fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
            let Some((identity, class, before, read_only)) = vmware_function(at) else {
                return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
            };
            let slot = &mut commands[usize::from(at.bus())][usize::from(at.device())]
                [usize::from(at.function())];
            match operation {
                HandoffAccess::ReadIdentity => Some(identity),
                HandoffAccess::ReadClass => Some(class),
                HandoffAccess::ReadCommand => Some(u32::from(slot.unwrap_or(before))),
                HandoffAccess::WriteCommand(value) => {
                    if !read_only {
                        *slot = Some(value);
                    }
                    Some(0)
                }
                HandoffAccess::QuiesceIsaDma => {
                    quiesces += 1;
                    Some(0)
                }
            }
        })
        .expect("the captured VMware segment admits its exact policy");

        assert_eq!(report.functions(), 43);
        assert_eq!(report.disabled(), 40);
        assert_eq!(report.retained_non_requesters(), 1);
        assert_eq!(report.quiesced_requesters(), 1);
        assert_eq!(quiesces, 1);
        assert!(report.take_non_requester().is_some());
        assert!(report.take_quiesced_requester().is_some());
        let requesters = report.functions() - report.retained_non_requesters();
        assert_eq!(requesters, 42);
        assert!(
            requesters as usize <= FUNCTION_AUTHORITY_LIMIT,
            "VMware's unit governs {requesters} requesters but only {FUNCTION_AUTHORITY_LIMIT} function authorities exist"
        );
    }

    #[test]
    fn only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester() {
        let isa = Address::new(0, 7, 0, 0).expect("VMware ISA bridge");
        let identity = 0x7110_8086;
        let class = 0x06_01_00_00;
        let command = command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER;

        let fence = |policy: HandoffFencePolicy,
                     at: Address,
                     identity: u32,
                     class: u32,
                     command: u16,
                     status: Option<u32>,
                     quiesces: &mut u32,
                     written: &mut Option<u16>| {
            fence_dma_handoff(policy, |candidate, operation| {
                if candidate.function_base() != at {
                    return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
                }
                match operation {
                    HandoffAccess::ReadIdentity => Some(identity),
                    HandoffAccess::ReadClass => Some(class),
                    HandoffAccess::ReadCommand => Some(u32::from(command)),
                    HandoffAccess::WriteCommand(value) => {
                        *written = Some(value);
                        Some(0)
                    }
                    HandoffAccess::QuiesceIsaDma => {
                        *quiesces += 1;
                        status
                    }
                }
            })
        };

        let (mut quiesces, mut written) = (0, None);
        let mut report = fence(
            HandoffFencePolicy::vmware_440bx(),
            isa,
            identity,
            class,
            command,
            Some(0x0f0f),
            &mut quiesces,
            &mut written,
        )
        .expect("the exact ISA bridge is quiesced at its 8237 controllers");
        assert_eq!(report.functions(), 1);
        assert_eq!(report.disabled(), 0);
        assert_eq!(report.retained_non_requesters(), 0);
        assert_eq!(report.quiesced_requesters(), 1);
        assert_eq!(quiesces, 1);
        assert_eq!(written, Some(command::IO_SPACE | command::MEMORY_SPACE));

        let authority = report
            .take_quiesced_requester()
            .expect("entry fence retained its exact quiesce authority");
        assert_eq!(authority.address(), isa);
        let mut topology = PciTopology::new();
        topology
            .observe_quiesced_requester(
                authority,
                FunctionObservation::new(
                    isa,
                    identity,
                    class,
                    command,
                    Class::from_bits(class),
                    HeaderType::from_bits(0),
                    None,
                ),
            )
            .expect("quiesced requester topology observation");
        assert_eq!(topology.quiesced_requester(isa), None);
        topology.seal().expect("root-bus topology");
        assert!(topology.contains_function(isa));
        assert!(topology.contains_requester(isa));
        let record = topology
            .quiesced_requester(isa)
            .expect("the sealed snapshot names the quiesced requester");
        assert_eq!(
            (record.address(), record.identity(), record.command()),
            (isa, identity, command)
        );
        let other = Address::new(0, 7, 1, 0).expect("IDE function");
        assert_eq!(topology.quiesced_requester(other), None);

        for (at, later_identity, later_class, later_command) in [
            (other, identity, class, command),
            (isa, identity ^ 1, class, command),
            (isa, identity, class ^ 1, command),
            (isa, identity, class, command & !command::IO_SPACE),
        ] {
            let (mut quiesces, mut written) = (0, None);
            let changed = fence(
                HandoffFencePolicy::vmware_440bx(),
                isa,
                identity,
                class,
                command,
                Some(0),
                &mut quiesces,
                &mut written,
            )
            .expect("exact ISA bridge")
            .take_quiesced_requester()
            .expect("exact quiesce authority");
            assert_eq!(
                PciTopology::new().observe_quiesced_requester(
                    changed,
                    FunctionObservation::new(
                        at,
                        later_identity,
                        later_class,
                        later_command,
                        Class::from_bits(later_class),
                        HeaderType::from_bits(0),
                        None,
                    ),
                ),
                Err(TopologyError::QuiescedRequesterChanged {
                    at: at.function_base(),
                })
            );
        }

        for revision in [0u32, 1, u32::from(u8::MAX)] {
            let (mut quiesces, mut written) = (0, None);
            let report = fence(
                HandoffFencePolicy::vmware_440bx(),
                isa,
                identity,
                class | revision,
                command,
                Some(0),
                &mut quiesces,
                &mut written,
            )
            .expect("every PIIX4 revision is the same ISA bridge");
            assert_eq!(report.quiesced_requesters(), 1);
        }

        for (at, identity, class, command) in [
            (isa, identity ^ 1, class, command),
            (isa, identity ^ (1 << 16), class, command),
            (isa, identity, class ^ (1 << 24), command),
            (isa, identity, class ^ (1 << 16), command),
            (isa, identity, class ^ (1 << 8), command),
            (isa, identity, class, command & !command::IO_SPACE),
            (isa, identity, class, command & !command::MEMORY_SPACE),
            (
                Address::new(1, 7, 0, 0).expect("nearby bus"),
                identity,
                class,
                command,
            ),
            (
                Address::new(0, 6, 0, 0).expect("nearby device"),
                identity,
                class,
                command,
            ),
            (other, identity, class, command),
        ] {
            let (mut quiesces, mut written) = (0, None);
            let refused = fence(
                HandoffFencePolicy::vmware_440bx(),
                at,
                identity,
                class,
                command,
                Some(0),
                &mut quiesces,
                &mut written,
            );
            assert!(matches!(
                refused,
                Err(HandoffFenceError::CommandRefused { .. })
            ));
            assert_eq!(
                quiesces, 0,
                "a near miss never touches the 8237 controllers"
            );
        }

        let (mut quiesces, mut written) = (0, None);
        let strict = fence(
            HandoffFencePolicy::strict(),
            isa,
            identity,
            class,
            command,
            Some(0),
            &mut quiesces,
            &mut written,
        );
        assert!(matches!(
            strict,
            Err(HandoffFenceError::CommandRefused { .. })
        ));
        assert_eq!(quiesces, 0);
        assert!(!HandoffFencePolicy::strict().quiesces_isa_dma());

        // The word before the clear and the word read back are each exact:
        // one may not stand in for the other.
        for (first, second) in [
            (command & !command::IO_SPACE, command),
            (command, command & !command::IO_SPACE),
        ] {
            let mut reads = 0;
            let mut quiesces = 0;
            let refused = fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
                if at.function_base() != isa {
                    return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
                }
                match operation {
                    HandoffAccess::ReadIdentity => Some(identity),
                    HandoffAccess::ReadClass => Some(class),
                    HandoffAccess::ReadCommand => {
                        reads += 1;
                        Some(u32::from(if reads == 1 { first } else { second }))
                    }
                    HandoffAccess::WriteCommand(_) => Some(0),
                    HandoffAccess::QuiesceIsaDma => {
                        quiesces += 1;
                        Some(0)
                    }
                }
            });
            assert!(matches!(
                refused,
                Err(HandoffFenceError::CommandRefused { .. })
            ));
            assert_eq!(quiesces, 0);
        }

        for pending in [0x0010, 0x0080, 0x1000, 0x8000, 0xf0f0] {
            let (mut quiesces, mut written) = (0, None);
            let refused = fence(
                HandoffFencePolicy::vmware_440bx(),
                isa,
                identity,
                class,
                command,
                Some(pending),
                &mut quiesces,
                &mut written,
            );
            assert_eq!(
                refused.err(),
                Some(HandoffFenceError::IsaDmaPending {
                    at: isa,
                    status: pending,
                })
            );
        }

        let (mut quiesces, mut written) = (0, None);
        let unavailable = fence(
            HandoffFencePolicy::vmware_440bx(),
            isa,
            identity,
            class,
            command,
            None,
            &mut quiesces,
            &mut written,
        );
        assert_eq!(
            unavailable.err(),
            Some(HandoffFenceError::Access {
                at: isa,
                operation: HandoffAccess::QuiesceIsaDma,
            })
        );
        assert_eq!(
            HandoffFenceError::IsaDmaPending {
                at: isa,
                status: 0x0010,
            }
            .to_string(),
            "00:07.0 8237 status 0x0010 still requests DMA after masking"
        );
    }

    #[test]
    fn only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command() {
        let host = Address::new(0, 0, 0, 0).expect("VMware host bridge");
        let identity = 0x7190_8086;
        let class = 0x06_00_00_00;
        let command = command::MEMORY_SPACE | command::BUS_MASTER;
        let mut written = None;

        let mut report = fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
            if at.function_base() != host {
                return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
            }
            match operation {
                HandoffAccess::ReadIdentity => Some(identity),
                HandoffAccess::ReadClass => Some(class),
                HandoffAccess::ReadCommand => Some(u32::from(command)),
                HandoffAccess::WriteCommand(value) => {
                    written = Some(value);
                    Some(0)
                }
                HandoffAccess::QuiesceIsaDma => None,
            }
        })
        .expect("the qualified host bridge is not a PCI DMA requester");

        assert_eq!(report.functions(), 1);
        assert_eq!(report.disabled(), 0);
        assert_eq!(report.retained_non_requesters(), 1);
        assert_eq!(written, Some(command::MEMORY_SPACE));

        for revision in [0u32, 1, u32::from(u8::MAX)] {
            assert!(
                HandoffFencePolicy::vmware_440bx()
                    .classify_non_requester(host, identity, class | revision, command, command,)
                    .is_some()
            );
        }

        let authority = report
            .take_non_requester()
            .expect("entry fence retained its exact classification authority");
        assert_eq!(report.retained_non_requesters(), 1);
        let mut topology = PciTopology::new();
        topology
            .observe_non_requester(
                authority,
                FunctionObservation::new(
                    host,
                    identity,
                    class,
                    command,
                    Class::from_bits(class),
                    HeaderType::from_bits(0),
                    None,
                ),
            )
            .expect("non-requester topology observation");
        topology.seal().expect("root-bus topology");
        assert!(topology.contains_function(host));
        assert!(!topology.contains_requester(host));

        for (at, later_identity, later_class, later_command) in [
            (
                Address::new(0, 1, 0, 0).expect("different BDF"),
                identity,
                class,
                command,
            ),
            (host, identity ^ 1, class, command),
            (host, identity, class ^ 1, command),
            (host, identity, class, command | command::IO_SPACE),
        ] {
            let changed_authority = HandoffFencePolicy::vmware_440bx()
                .classify_non_requester(host, identity, class, command, command)
                .expect("exact host bridge classification");
            let mut changed_topology = PciTopology::new();
            assert_eq!(
                changed_topology.observe_non_requester(
                    changed_authority,
                    FunctionObservation::new(
                        at,
                        later_identity,
                        later_class,
                        later_command,
                        Class::from_bits(later_class),
                        HeaderType::from_bits(0),
                        None,
                    ),
                ),
                Err(TopologyError::QualifiedNonRequesterChanged {
                    at: at.function_base(),
                })
            );
        }

        for (at, identity, class, command) in [
            (host, identity ^ 1, class, command),
            (host, identity ^ (1 << 16), class, command),
            (host, identity, class ^ (1 << 24), command),
            (host, identity, class ^ (1 << 16), command),
            (host, identity, class ^ (1 << 8), command),
            (host, identity, class, command | command::IO_SPACE),
            (
                Address::new(1, 0, 0, 0).expect("nearby bus"),
                identity,
                class,
                command,
            ),
            (
                Address::new(0, 1, 0, 0).expect("nearby function"),
                identity,
                class,
                command,
            ),
            (
                Address::new(0, 0, 1, 0).expect("nearby function"),
                identity,
                class,
                command,
            ),
        ] {
            let refused = fence_dma_handoff(
                HandoffFencePolicy::vmware_440bx(),
                |candidate, operation| {
                    if candidate.function_base() != at {
                        return matches!(operation, HandoffAccess::ReadIdentity)
                            .then_some(u32::MAX);
                    }
                    match operation {
                        HandoffAccess::ReadIdentity => Some(identity),
                        HandoffAccess::ReadClass => Some(class),
                        HandoffAccess::ReadCommand => Some(u32::from(command)),
                        HandoffAccess::WriteCommand(_) => Some(0),
                        HandoffAccess::QuiesceIsaDma => None,
                    }
                },
            );
            assert!(matches!(
                refused,
                Err(HandoffFenceError::CommandRefused { .. })
            ));
        }

        let strict = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
            if at.function_base() != host {
                return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
            }
            match operation {
                HandoffAccess::ReadIdentity => Some(identity),
                HandoffAccess::ReadClass => Some(class),
                HandoffAccess::ReadCommand => Some(u32::from(command)),
                HandoffAccess::WriteCommand(_) => Some(0),
                HandoffAccess::QuiesceIsaDma => None,
            }
        });
        assert!(matches!(
            strict,
            Err(HandoffFenceError::CommandRefused { .. })
        ));

        let mut unexpected_before_reads = 0;
        let unexpected_before =
            fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
                if at.function_base() != host {
                    return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
                }
                match operation {
                    HandoffAccess::ReadIdentity => Some(identity),
                    HandoffAccess::ReadClass => Some(class),
                    HandoffAccess::ReadCommand => {
                        unexpected_before_reads += 1;
                        Some(u32::from(if unexpected_before_reads == 1 {
                            command | command::IO_SPACE
                        } else {
                            command
                        }))
                    }
                    HandoffAccess::WriteCommand(_) => Some(0),
                    HandoffAccess::QuiesceIsaDma => None,
                }
            });
        assert!(matches!(
            unexpected_before,
            Err(HandoffFenceError::CommandRefused { .. })
        ));

        let mut command_reads = 0;
        let changed_command =
            fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
                if at.function_base() != host {
                    return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
                }
                match operation {
                    HandoffAccess::ReadIdentity => Some(identity),
                    HandoffAccess::ReadClass => Some(class),
                    HandoffAccess::ReadCommand => {
                        command_reads += 1;
                        Some(u32::from(if command_reads == 1 {
                            command
                        } else {
                            command | command::IO_SPACE
                        }))
                    }
                    HandoffAccess::WriteCommand(_) => Some(0),
                    HandoffAccess::QuiesceIsaDma => None,
                }
            });
        assert!(matches!(
            changed_command,
            Err(HandoffFenceError::CommandRefused { .. })
        ));

        let mut class_reads = 0;
        let changed_class =
            fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
                if at.function_base() != host {
                    return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
                }
                match operation {
                    HandoffAccess::ReadIdentity => Some(identity),
                    HandoffAccess::ReadClass => {
                        class_reads += 1;
                        Some(if class_reads == 1 { class } else { class ^ 1 })
                    }
                    HandoffAccess::ReadCommand => Some(u32::from(command)),
                    HandoffAccess::WriteCommand(_) => Some(0),
                    HandoffAccess::QuiesceIsaDma => None,
                }
            });
        assert!(matches!(
            changed_class,
            Err(HandoffFenceError::ClassChanged {
                at,
                expected,
                found,
            }) if at == host && expected == class && found == class ^ 1
        ));
    }

    const TRACE_CAPACITY: usize = 96;
    const COMMAND_TRACE_CAPACITY: usize = 16;

    struct FakeCommand {
        command: u16,
        status: u16,
        writable: u16,
        injected_read: Option<(usize, u16)>,
        trace: [Option<CommandAccess>; COMMAND_TRACE_CAPACITY],
        calls: usize,
        fail_at: Option<usize>,
    }

    impl FakeCommand {
        fn new(command: u16, writable: u16) -> Self {
            Self {
                command,
                status: 0xA55A,
                writable,
                injected_read: None,
                trace: [None; COMMAND_TRACE_CAPACITY],
                calls: 0,
                fail_at: None,
            }
        }

        fn access(&mut self, event: CommandAccess) -> Option<u32> {
            let call = self.calls;
            self.trace[call] = Some(event);
            self.calls += 1;
            if self.fail_at == Some(call) {
                return None;
            }

            match event {
                CommandAccess::Read => {
                    let injected = self
                        .injected_read
                        .filter(|(at, _)| *at == call)
                        .map_or(0, |(_, bits)| bits);
                    Some(u32::from(self.command | injected) | (u32::from(self.status) << 16))
                }
                CommandAccess::Write(value) => {
                    self.command = (self.command & !self.writable) | (value & self.writable);
                    Some(0)
                }
            }
        }

        fn events(&self) -> &[Option<CommandAccess>] {
            &self.trace[..self.calls]
        }
    }

    #[derive(Clone)]
    struct FakeConfig {
        command: u16,
        status: u16,
        bars: [u32; BARS],
        masks: [u32; BARS],
        trace: [Option<ProbeAccess>; TRACE_CAPACITY],
        calls: usize,
        fail_at: Option<usize>,
        ignore_write_at: Option<usize>,
        bar_writes_with_bus_master: usize,
    }

    impl FakeConfig {
        fn device() -> Self {
            Self {
                command: command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER,
                status: 0xA55A,
                bars: [0x8000_0004, 1, 0x0000_C001, 0, 0, 0],
                masks: [0xFFFF_C004, u32::MAX, 0xFFFF_FFE1, 0, 0, 0],
                trace: [None; TRACE_CAPACITY],
                calls: 0,
                fail_at: None,
                ignore_write_at: None,
                bar_writes_with_bus_master: 0,
            }
        }

        fn access(&mut self, event: ProbeAccess) -> Result<u32, usize> {
            let call = self.calls;
            self.trace[call] = Some(event);
            self.calls += 1;

            let answer = match event {
                ProbeAccess::ReadCommand => {
                    u32::from(self.command) | (u32::from(self.status) << 16)
                }
                ProbeAccess::WriteCommand(command) => {
                    if self.ignore_write_at != Some(call) {
                        self.command = command;
                    }
                    0
                }
                ProbeAccess::ReadBar(index) => {
                    if self.bars[index] == u32::MAX {
                        self.masks[index]
                    } else {
                        self.bars[index]
                    }
                }
                ProbeAccess::WriteBar(index, value) => {
                    if self.command & command::BUS_MASTER != 0 {
                        self.bar_writes_with_bus_master += 1;
                    }
                    if self.ignore_write_at != Some(call) {
                        self.bars[index] = value;
                    }
                    0
                }
            };
            if self.fail_at == Some(call) {
                Err(call)
            } else {
                Ok(answer)
            }
        }

        fn events(&self) -> &[Option<ProbeAccess>] {
            &self.trace[..self.calls]
        }
    }

    fn fake_command_access(fake: &mut FakeConfig, access: CommandAccess) -> Option<u32> {
        let event = match access {
            CommandAccess::Read => ProbeAccess::ReadCommand,
            CommandAccess::Write(value) => ProbeAccess::WriteCommand(value),
        };
        fake.access(event).ok()
    }

    fn command_address(function: u8) -> Address {
        Address::new(0, 1, function, header::BAR0).expect("a valid test function")
    }

    fn authority(at: Address, header: HeaderType) -> FunctionAuthority {
        FunctionAuthority {
            lease: FunctionLease {
                registry: 1,
                at: at.function_base(),
                generation: 1,
            },
            registers: header
                .bar_registers()
                .expect("a test authority needs a supported header"),
        }
    }

    fn device_authority(at: Address) -> FunctionAuthority {
        authority(at, HeaderType::from_bits(0))
    }

    fn state_authority(state: &CommandState) -> FunctionAuthority {
        FunctionAuthority {
            lease: state.lease.function,
            registers: BARS,
        }
    }

    fn enable_bus_master<const N: usize>(
        leases: &mut CommandLeases<N>,
        at: Address,
        access: impl FnMut(Address, CommandAccess) -> Option<u32>,
    ) -> Result<CommandState, EnableFailure> {
        super::enable_bus_master_with_leases(leases, &device_authority(at), access)
    }

    fn enable_memory_space<const N: usize>(
        leases: &mut CommandLeases<N>,
        at: Address,
        access: impl FnMut(Address, CommandAccess) -> Option<u32>,
    ) -> Result<CommandState, EnableFailure> {
        super::enable_memory_space_with_leases(leases, &device_authority(at), access)
    }

    fn admit_bus_master<const N: usize>(
        leases: &mut CommandLeases<N>,
        state: &mut CommandState,
        access: impl FnMut(Address, CommandAccess) -> Option<u32>,
    ) -> Result<(), EnableError> {
        super::admit_bus_master_with_leases(leases, state.lease.function, state, access)
    }

    fn disable_bus_master_for_retirement<const N: usize>(
        leases: &mut CommandLeases<N>,
        state: &mut CommandState,
        access: impl FnMut(Address, CommandAccess) -> Option<u32>,
    ) -> Result<(), EnableError> {
        super::disable_bus_master_for_retirement_with_leases(
            leases,
            state.lease.function,
            state,
            access,
        )
    }

    fn restore_command<const N: usize>(
        leases: &mut CommandLeases<N>,
        state: CommandState,
        access: impl FnMut(Address, CommandAccess) -> Option<u32>,
    ) -> Result<(), RestoreFailure> {
        let authority = state_authority(&state);
        super::restore_command_with_leases(leases, &authority, state, access)
    }

    fn commit_command<const N: usize>(
        leases: &mut CommandLeases<N>,
        state: CommandState,
        access: impl FnMut(Address, CommandAccess) -> Option<u32>,
    ) -> Result<(), RestoreFailure> {
        let authority = state_authority(&state);
        super::commit_command_with_leases(leases, &authority, state, access)
    }

    fn probe_bars<E>(
        registers: usize,
        mut access: impl FnMut(ProbeAccess) -> Result<u32, E>,
    ) -> Result<ProbedBars, ProbeError<E>> {
        let staged =
            probe_bars_unleased(registers, &mut access).map_err(|failure| failure.reason)?;
        restore_staged_probe(staged, &mut access).map_err(|failure| failure.reason)
    }

    fn duplicate_state(state: &CommandState) -> CommandState {
        CommandState {
            lease: state.lease,
            before: state.before,
            installed: state.installed,
            phase: state.phase,
        }
    }

    #[test]
    fn command_state_is_bound_to_its_registry_function_and_command_offset() {
        let supplied = command_address(0);
        let expected = supplied
            .function_base()
            .at(header::COMMAND)
            .expect("the command register is aligned");
        let other = command_address(1)
            .function_base()
            .at(header::COMMAND)
            .expect("the command register is aligned");
        let mut first_fake = FakeCommand::new(0, u16::MAX);
        let mut second_fake = FakeCommand::new(0, u16::MAX);
        let mut first = CommandLeases::<1>::new();
        let mut second = CommandLeases::<1>::new();

        let first_state = enable_bus_master(&mut first, supplied, |at, event| {
            assert_eq!(at, expected);
            first_fake.access(event)
        })
        .expect("the first registry enabled its function");
        let second_state = enable_bus_master(&mut second, supplied, |at, event| {
            assert_eq!(at, expected);
            second_fake.access(event)
        })
        .expect("the second registry enabled its function");
        assert_eq!(first_state.lease.generation, second_state.lease.generation);
        assert_ne!(first_state.lease.registry, second_state.lease.registry);

        let mut wrong_function = duplicate_state(&first_state);
        wrong_function.lease.function.at = other.function_base();
        let mut accesses = 0;
        let wrong_function = commit_command(&mut first, wrong_function, |_, _| {
            accesses += 1;
            None
        })
        .expect_err("the token cannot be aimed at another function");
        assert_eq!(wrong_function.reason(), EnableError::Stale);
        assert_eq!(accesses, 0);

        let mut accesses = 0;
        let cross_registry = commit_command(&mut second, first_state, |_, _| {
            accesses += 1;
            None
        })
        .expect_err("another registry cannot consume the token");
        assert_eq!(cross_registry.reason(), EnableError::Stale);
        assert_eq!(accesses, 0);

        commit_command(&mut first, cross_registry.into_state(), |at, event| {
            assert_eq!(at, expected);
            first_fake.access(event)
        })
        .expect("the issuing registry retained its token");
        commit_command(&mut second, second_state, |at, event| {
            assert_eq!(at, expected);
            second_fake.access(event)
        })
        .expect("the other registry retained its own token");
    }

    #[test]
    fn command_leases_refuse_nesting_and_preserve_uncertain_retry_authority() {
        let at = command_address(0);
        let mut fake = FakeCommand::new(0, !command::BUS_MASTER);
        fake.fail_at = Some(4);
        let mut leases = CommandLeases::<2>::new();

        let failure = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
            .expect_err("the final rollback read was unavailable");
        assert_eq!(failure.reason(), EnableError::Restore);
        let state = failure
            .into_state()
            .expect("an uncertain write retains cleanup authority");
        let calls = fake.calls;
        let nested = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
            .expect_err("a live command transition is exclusive");
        assert_eq!(nested.reason(), EnableError::Leased);
        assert_eq!(nested.into_state(), None);
        assert_eq!(fake.calls, calls, "nesting performed an access");
        assert_eq!(
            leases.begin_probe(&device_authority(at)),
            Err(EnableError::Leased)
        );

        fake.fail_at = None;
        restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect("the same uncertain token retries exactly");
        fake.writable = u16::MAX;
        let replacement = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
            .expect("exact restoration made room again");
        commit_command(&mut leases, replacement, |_, event| fake.access(event))
            .expect("the replacement remained current");
    }

    #[test]
    fn command_consumption_requires_the_installed_word_and_live_generation() {
        let at = command_address(0);
        let installed = command::MEMORY_SPACE | command::BUS_MASTER;
        let mut fake = FakeCommand::new(0, u16::MAX);
        let mut leases = CommandLeases::<1>::new();

        let first = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
            .expect("the first generation was installed");
        let stale = duplicate_state(&first);
        commit_command(&mut leases, first, |_, event| fake.access(event))
            .expect("the first generation was committed");
        let current = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
            .expect("the next generation was reserved");

        let calls = fake.calls;
        let stale = commit_command(&mut leases, stale, |_, event| fake.access(event))
            .expect_err("a released generation cannot consume its replacement");
        assert_eq!(stale.reason(), EnableError::Stale);
        assert_eq!(fake.calls, calls, "a stale token reached the device");

        fake.command = installed | command::IO_SPACE;
        let calls = fake.calls;
        let changed = commit_command(&mut leases, current, |_, event| fake.access(event))
            .expect_err("a changed command word cannot be committed");
        assert_eq!(changed.reason(), EnableError::Changed(fake.command));
        assert_eq!(fake.calls, calls + 1);
        assert_eq!(fake.events()[calls], Some(CommandAccess::Read));

        fake.command = installed;
        commit_command(&mut leases, changed.into_state(), |_, event| {
            fake.access(event)
        })
        .expect("the unchanged token retries after the exact word returns");

        fake.command = 0;
        let restorable = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
            .expect("a restorable generation was installed");
        fake.command = installed | command::IO_SPACE;
        let calls = fake.calls;
        let changed = restore_command(&mut leases, restorable, |_, event| fake.access(event))
            .expect_err("restoration cannot overwrite an independently changed word");
        assert_eq!(changed.reason(), EnableError::Changed(fake.command));
        assert_eq!(fake.calls, calls + 1);
        assert_eq!(fake.events()[calls], Some(CommandAccess::Read));

        fake.command = installed;
        restore_command(&mut leases, changed.into_state(), |_, event| {
            fake.access(event)
        })
        .expect("restoration retries with the same token");
        assert_eq!(fake.command, 0);
    }

    #[test]
    fn command_generation_exhaustion_refuses_before_configuration_access() {
        let mut leases = CommandLeases::<1>::new();
        leases.generation = u64::MAX;
        let mut accessed = false;

        let failure = enable_bus_master(&mut leases, command_address(0), |_, _| {
            accessed = true;
            Some(0)
        })
        .expect_err("a command generation cannot wrap");
        assert_eq!(failure.reason(), EnableError::Exhausted);
        assert_eq!(failure.into_state(), None);
        assert!(!accessed);
        assert!(leases.slots.iter().all(Option::is_none));
    }

    #[test]
    fn a_command_upgrade_cannot_cross_command_registries() {
        let at = command_address(0);
        let mut fake = FakeCommand::new(0, u16::MAX);
        let mut issuing = CommandLeases::<1>::new();
        let mut other = CommandLeases::<1>::new();
        let mut state = enable_memory_space(&mut issuing, at, |_, event| fake.access(event))
            .expect("the issuing registry installed memory decode");
        let calls = fake.calls;

        assert_eq!(
            admit_bus_master(&mut other, &mut state, |_, event| fake.access(event)),
            Err(EnableError::Stale)
        );
        assert_eq!(fake.calls, calls, "a foreign registry reached the device");

        admit_bus_master(&mut issuing, &mut state, |_, event| fake.access(event))
            .expect("the issuing registry retained its upgrade authority");
        restore_command(&mut issuing, state, |_, event| fake.access(event))
            .expect("the issuing registry retained its restoration authority");
    }

    #[test]
    fn a_clean_upgrade_refusal_can_retry_the_same_state() {
        let at = command_address(0);
        let mut fake = FakeCommand::new(0, u16::MAX);
        let mut leases = CommandLeases::<1>::new();
        let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
            .expect("memory decode was installed");
        fake.fail_at = Some(fake.calls + 1);

        assert_eq!(
            admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event)),
            Err(EnableError::Access)
        );
        assert_eq!(state.phase, CommandPhase::Installed);
        assert_eq!(fake.command, command::MEMORY_SPACE);

        fake.fail_at = None;
        admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
            .expect("exact rollback left the same state retryable");
        restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect("the retried upgrade retained its original word");
        assert_eq!(fake.command, 0);
    }

    #[test]
    fn every_bus_master_upgrade_access_refusal_preserves_its_owner() {
        let at = command_address(0);
        let memory_only = command::MEMORY_SPACE;

        for fail_at in 0..=2 {
            let mut fake = FakeCommand::new(0, u16::MAX);
            let mut leases = CommandLeases::<1>::new();
            let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
                .expect("memory decode stage");
            fake.calls = 0;
            fake.trace = [None; COMMAND_TRACE_CAPACITY];
            fake.fail_at = Some(fail_at);

            assert_eq!(
                admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event)),
                Err(EnableError::Access),
                "access {fail_at}"
            );
            assert_eq!(fake.command, memory_only, "access {fail_at}");
            assert_eq!(state.phase, CommandPhase::Installed, "access {fail_at}");
            assert_eq!(
                leases.begin_probe(&device_authority(at)),
                Err(EnableError::Leased),
                "access {fail_at} lost the command owner"
            );

            fake.injected_read = None;
            fake.fail_at = None;
            admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
                .expect("the unchanged owner retries the upgrade");
            restore_command(&mut leases, state, |_, event| fake.access(event))
                .expect("the retried owner restores the original command");
            assert_eq!(fake.command, 0, "access {fail_at}");
        }
    }

    #[test]
    fn failed_bus_master_upgrade_rollback_never_loses_cleanup_authority() {
        let at = command_address(0);

        for fail_at in [3, 4] {
            let mut fake = FakeCommand::new(0, !command::BUS_MASTER);
            let mut leases = CommandLeases::<1>::new();
            let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
                .expect("memory decode stage");
            fake.calls = 0;
            fake.trace = [None; COMMAND_TRACE_CAPACITY];
            fake.fail_at = Some(fail_at);

            let failure = admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
                .expect_err("the device refuses bus master");
            assert_eq!(fake.command, command::MEMORY_SPACE, "access {fail_at}");
            assert_eq!(
                leases.begin_probe(&device_authority(at)),
                Err(EnableError::Leased),
                "access {fail_at} lost the command owner"
            );

            if fail_at == 3 {
                assert_eq!(failure, EnableError::Refused(command::MEMORY_SPACE));
                assert_eq!(state.phase, CommandPhase::Installed);
            } else {
                assert_eq!(failure, EnableError::Restore);
                assert_eq!(
                    state.phase,
                    CommandPhase::Restoring(command::MEMORY_SPACE | command::BUS_MASTER)
                );
            }

            fake.fail_at = None;
            fake.writable = u16::MAX;
            restore_command(&mut leases, state, |_, event| fake.access(event))
                .expect("the retained owner restores the original command");
            assert_eq!(fake.command, 0, "access {fail_at}");
        }
    }

    #[test]
    fn bus_master_upgrade_rejects_another_function_before_access() {
        let mut functions = FunctionClaims::<2>::new();
        let first = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("first function authority");
        let second = functions
            .claim(command_address(1), HeaderType::from_bits(0))
            .expect("second function authority");
        let mut fake = FakeCommand::new(0, u16::MAX);
        let mut state =
            super::enable_memory_space(&mut functions, &first, |_, event| fake.access(event))
                .expect("first function memory decode");
        let calls = fake.calls;

        assert_eq!(
            super::admit_bus_master(&mut functions, &second, &mut state, |_, event| {
                fake.access(event)
            }),
            Err(EnableError::Stale)
        );
        assert_eq!(fake.calls, calls, "a foreign function reached the device");

        super::admit_bus_master(&mut functions, &first, &mut state, |_, event| {
            fake.access(event)
        })
        .expect("the issuing function retained the upgrade owner");
        super::restore_command(&mut functions, &first, state, |_, event| fake.access(event))
            .expect("the issuing function restores its command");
        release_function(&mut functions, first).expect("first release");
        release_function(&mut functions, second).expect("second release");
    }

    #[test]
    fn an_uncertain_command_state_cannot_be_committed() {
        let at = command_address(0);
        let mut fake = FakeCommand::new(0, !command::BUS_MASTER);
        fake.fail_at = Some(4);
        let mut leases = CommandLeases::<1>::new();
        let failure = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
            .expect_err("the rollback read was unavailable");
        assert_eq!(failure.reason(), EnableError::Restore);
        let state = failure
            .into_state()
            .expect("uncertainty retains the command owner");
        let calls = fake.calls;

        let failure = commit_command(&mut leases, state, |_, event| fake.access(event))
            .expect_err("uncertain hardware cannot be committed");
        assert_eq!(failure.reason(), EnableError::Stale);
        assert_eq!(fake.calls, calls, "commit inspected uncertain hardware");

        fake.fail_at = None;
        fake.writable = u16::MAX;
        restore_command(&mut leases, failure.into_state(), |_, event| {
            fake.access(event)
        })
        .expect("the refused commit returned exact cleanup authority");
    }

    #[test]
    fn restoration_retains_the_prewrite_word_across_access_loss() {
        let at = command_address(0);
        let mut fake = FakeCommand::new(0, u16::MAX);
        let mut leases = CommandLeases::<1>::new();
        let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
            .expect("memory decode was installed");
        let wanted = command::MEMORY_SPACE | command::BUS_MASTER;
        fake.injected_read = Some((fake.calls + 2, command::IO_SPACE));
        fake.fail_at = Some(fake.calls + 3);

        assert_eq!(
            admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event)),
            Err(EnableError::Restore)
        );
        assert_eq!(fake.command, wanted);
        assert_eq!(state.phase, CommandPhase::Restoring(wanted));

        fake.injected_read = None;
        fake.fail_at = Some(fake.calls + 1);
        let failure = restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect_err("the first restoration write was unavailable");
        assert_eq!(failure.reason(), EnableError::Refused(wanted));
        assert_eq!(fake.command, wanted);

        fake.fail_at = None;
        restore_command(&mut leases, failure.into_state(), |_, event| {
            fake.access(event)
        })
        .expect("the same token retained both possible hardware words");
        assert_eq!(fake.command, 0);
    }

    #[test]
    fn an_already_original_word_releases_after_its_readback() {
        let at = command_address(0);
        let required = command::MEMORY_SPACE | command::BUS_MASTER;
        let mut fake = FakeCommand::new(required, u16::MAX);
        let mut leases = CommandLeases::<1>::new();
        let state = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
            .expect("the already enabled command was leased");

        restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect("the captured word already read back exactly");
        assert_eq!(
            fake.events(),
            &[Some(CommandAccess::Read), Some(CommandAccess::Read)]
        );
    }

    #[test]
    fn an_interrupted_upgrade_records_retry_authority_before_the_write() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let at = command_address(0);
        let mut fake = FakeCommand::new(0, u16::MAX);
        let mut leases = CommandLeases::<1>::new();
        let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
            .expect("memory decode was installed first");
        let installed = state.installed;
        let wanted = installed | command::BUS_MASTER;

        let interrupted = catch_unwind(AssertUnwindSafe(|| {
            let _ = admit_bus_master(&mut leases, &mut state, |_, event| match event {
                CommandAccess::Read => fake.access(event),
                CommandAccess::Write(_) => panic!("injected interruption before the write"),
            });
        }));
        assert!(interrupted.is_err());
        assert_eq!(state.phase, CommandPhase::Restoring(wanted));
        assert_eq!(fake.command, installed);
        assert_eq!(
            leases.begin_probe(&device_authority(at)),
            Err(EnableError::Leased)
        );

        restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect("the pre-recorded state retained exact restoration authority");
        assert_eq!(fake.command, 0);
    }

    #[test]
    fn bar_probe_reservations_are_registry_bound() {
        let at = command_address(0);
        let authority = device_authority(at);
        let mut first = CommandLeases::<1>::new();
        let mut second = CommandLeases::<1>::new();
        let first_probe = first.begin_probe(&authority).expect("first reservation");
        let second_probe = second.begin_probe(&authority).expect("second reservation");

        let first_probe = second
            .finish_probe(&authority, first_probe)
            .expect_err("another registry cannot release the reservation");
        assert_eq!(second.begin_probe(&authority), Err(EnableError::Leased));
        first
            .finish_probe(&authority, first_probe)
            .expect("the issuing registry releases it");
        second
            .finish_probe(&authority, second_probe)
            .expect("the other registry retained its reservation");
        let first_probe = first
            .begin_probe(&authority)
            .expect("first registry made room");
        let second_probe = second
            .begin_probe(&authority)
            .expect("second registry made room");
        first
            .finish_probe(&authority, first_probe)
            .expect("first exact release");
        second
            .finish_probe(&authority, second_probe)
            .expect("second exact release");
    }

    #[test]
    fn one_registry_identity_binds_every_simultaneous_lease() {
        let mut leases = CommandLeases::<2>::new();
        let first_authority = device_authority(command_address(0));
        let second_authority = device_authority(command_address(1));
        let first = leases
            .begin_probe(&first_authority)
            .expect("the first function was reserved");
        let second = leases
            .begin_probe(&second_authority)
            .expect("the second function was reserved");

        assert_ne!(first.0.registry, 0);
        assert_eq!(second.0.registry, first.0.registry);

        leases
            .finish_probe(&first_authority, first)
            .expect("first release");
        leases
            .finish_probe(&second_authority, second)
            .expect("second release");
    }

    #[test]
    fn bus_master_enable_is_verified_and_preserves_the_command() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | OTHER;
        let wanted = original | command::MEMORY_SPACE | command::BUS_MASTER;
        let mut fake = FakeCommand::new(original, u16::MAX);
        let mut leases = CommandLeases::<1>::new();

        let state = enable_bus_master(&mut leases, command_address(0), |_, event| {
            fake.access(event)
        })
        .expect("enabled");
        assert_eq!(fake.command, wanted);
        assert_eq!(
            fake.events(),
            &[
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(wanted)),
                Some(CommandAccess::Read),
            ]
        );

        let mut already = FakeCommand::new(wanted, u16::MAX);
        let mut already_leases = CommandLeases::<1>::new();
        let already_state =
            enable_bus_master(&mut already_leases, command_address(0), |_, event| {
                already.access(event)
            })
            .expect("enabled");
        assert_eq!(already.events(), &[Some(CommandAccess::Read)]);

        restore_command(&mut leases, state, |_, event| fake.access(event)).expect("restored");
        assert_eq!(fake.command, original);
        restore_command(&mut already_leases, already_state, |_, event| {
            already.access(event)
        })
        .expect("restored");
        assert_eq!(already.command, wanted);
    }

    #[test]
    fn bus_master_enable_preserves_either_already_required_bit() {
        const OTHER: u16 = 1 << 9;
        const REQUIRED: u16 = command::MEMORY_SPACE | command::BUS_MASTER;

        for original in [OTHER | command::MEMORY_SPACE, OTHER | command::BUS_MASTER] {
            let mut fake = FakeCommand::new(original, u16::MAX);
            let mut leases = CommandLeases::<1>::new();
            let state = enable_bus_master(&mut leases, command_address(0), |_, event| {
                fake.access(event)
            })
            .expect("enabled");

            assert_eq!(fake.command, original | REQUIRED);
            restore_command(&mut leases, state, |_, event| fake.access(event)).expect("restored");
            assert_eq!(fake.command, original);
        }
    }

    #[test]
    fn memory_decode_and_bus_master_are_two_exact_restorable_stages() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | command::BUS_MASTER | OTHER;
        let mut fake = FakeCommand::new(original, u16::MAX);
        let mut leases = CommandLeases::<1>::new();

        let mut original_state =
            enable_memory_space(&mut leases, command_address(0), |_, event| {
                fake.access(event)
            })
            .expect("memory decode enabled");
        let memory_only = (original | command::MEMORY_SPACE) & !command::BUS_MASTER;
        assert_eq!(fake.command, memory_only);
        assert_eq!(fake.command & command::BUS_MASTER, 0);

        admit_bus_master(&mut leases, &mut original_state, |_, event| {
            fake.access(event)
        })
        .expect("bus master enabled later");
        assert_eq!(
            fake.command,
            original | command::MEMORY_SPACE | command::BUS_MASTER
        );

        restore_command(&mut leases, original_state, |_, event| fake.access(event))
            .expect("original restored");
        assert_eq!(fake.command, original);
        assert_eq!(
            fake.events(),
            &[
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(memory_only)),
                Some(CommandAccess::Read),
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(
                    original | command::MEMORY_SPACE | command::BUS_MASTER,
                )),
                Some(CommandAccess::Read),
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(original)),
                Some(CommandAccess::Read),
            ]
        );
    }

    #[test]
    fn retirement_disables_and_does_not_restore_inherited_bus_master() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | command::BUS_MASTER | OTHER;
        let safe_original = original & !command::BUS_MASTER;
        let memory_only = (original | command::MEMORY_SPACE) & !command::BUS_MASTER;
        let admitted = original | command::MEMORY_SPACE | command::BUS_MASTER;
        let mut fake = FakeCommand::new(original, u16::MAX);
        let mut leases = CommandLeases::<1>::new();
        let mut state = enable_memory_space(&mut leases, command_address(0), |_, event| {
            fake.access(event)
        })
        .expect("memory decode enabled with inherited BME withheld");
        admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
            .expect("isolated bus mastering admitted");

        disable_bus_master_for_retirement(&mut leases, &mut state, |_, event| fake.access(event))
            .expect("retirement fenced bus mastering");
        assert_eq!(fake.command, memory_only);
        assert_eq!(state.before, safe_original);

        restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect("all safe firmware command bits restored");
        assert_eq!(fake.command, safe_original);
        assert_eq!(fake.command & command::BUS_MASTER, 0);
        assert!(fake.events().windows(3).any(|events| events
            == [
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(admitted & !command::BUS_MASTER)),
                Some(CommandAccess::Read),
            ]));
    }

    #[test]
    fn unit_requester_fence_withholds_and_never_restores_inherited_bus_master() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | command::BUS_MASTER | OTHER;
        let safe = original & !command::BUS_MASTER;
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeCommand::new(original, u16::MAX);

        let state =
            super::fence_bus_master(&mut functions, &authority, |_, access| fake.access(access))
                .expect("requester fenced");
        assert_eq!(fake.command, safe);
        assert_eq!(state.before, safe);
        super::verify_bus_master_fence(&mut functions, &authority, &state, |_, access| {
            fake.access(access)
        })
        .expect("fresh exact fence readback");

        super::restore_command(&mut functions, &authority, state, |_, access| {
            fake.access(access)
        })
        .expect("safe command restoration");
        assert_eq!(fake.command, safe);
        super::release_function(&mut functions, authority).expect("function returned");
    }

    #[test]
    fn requester_fence_and_fresh_verification_retain_exact_failure_authority() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | command::BUS_MASTER | OTHER;

        for fail_at in 0..=2 {
            let mut functions = FunctionClaims::<1>::new();
            let authority = functions
                .claim(command_address(0), HeaderType::from_bits(0))
                .expect("function authority");
            let mut fake = FakeCommand::new(original, u16::MAX);
            fake.fail_at = Some(fail_at);
            let failure = super::fence_bus_master(&mut functions, &authority, |_, access| {
                fake.access(access)
            })
            .expect_err("injected fence refusal");
            fake.fail_at = None;
            if let Some(state) = failure.into_state() {
                super::restore_command(&mut functions, &authority, state, |_, access| {
                    fake.access(access)
                })
                .expect("uncertain fence restored by its exact owner");
            }
            super::release_function(&mut functions, authority).expect("function returned");
        }

        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeCommand::new(original, u16::MAX);
        let state =
            super::fence_bus_master(&mut functions, &authority, |_, access| fake.access(access))
                .expect("requester fenced");
        fake.command ^= command::IO_SPACE;
        assert!(matches!(
            super::verify_bus_master_fence(&mut functions, &authority, &state, |_, access| fake
                .access(access)),
            Err(EnableError::Changed(_))
        ));
        fake.command ^= command::IO_SPACE;
        super::restore_command(&mut functions, &authority, state, |_, access| {
            fake.access(access)
        })
        .expect("fence remains exactly retryable");
        super::release_function(&mut functions, authority).expect("function returned");
    }

    #[test]
    fn an_uncertain_retirement_transition_can_settle_disabled() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | command::BUS_MASTER | OTHER;
        let mut fake = FakeCommand::new(original, u16::MAX);
        let mut leases = CommandLeases::<1>::new();
        let mut state = enable_memory_space(&mut leases, command_address(0), |_, event| {
            fake.access(event)
        })
        .expect("memory decode enabled");
        let memory_only = fake.command;
        admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
            .expect("isolated bus mastering admitted");
        let admitted = fake.command;

        fake.calls = 0;
        fake.trace = [None; COMMAND_TRACE_CAPACITY];
        fake.injected_read = Some((2, 1 << 10));
        fake.fail_at = Some(3);
        assert_eq!(
            disable_bus_master_for_retirement(&mut leases, &mut state, |_, event| {
                fake.access(event)
            }),
            Err(EnableError::Restore)
        );
        assert_eq!(fake.command, memory_only);
        assert_eq!(state.installed, admitted);
        assert_eq!(state.phase, CommandPhase::Restoring(memory_only));
        assert_eq!(state.before, original);

        fake.injected_read = None;
        fake.fail_at = None;
        disable_bus_master_for_retirement(&mut leases, &mut state, |_, event| fake.access(event))
            .expect("the two recorded words settle at BME off");
        assert_eq!(fake.command, memory_only);
        assert_eq!(state.phase, CommandPhase::Installed);
        assert_eq!(state.before, original & !command::BUS_MASTER);
        restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect("the settled state remains exactly releasable");
        assert_eq!(fake.command, original & !command::BUS_MASTER);
    }

    #[test]
    fn every_retirement_access_refusal_retains_exact_retry_authority() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | command::BUS_MASTER | OTHER;

        for fail_at in 1..=4 {
            let mut functions = FunctionClaims::<1>::new();
            let authority = functions
                .claim(command_address(0), HeaderType::from_bits(0))
                .expect("function authority");
            let mut fake = FakeCommand::new(original, u16::MAX);
            let mut state = super::enable_memory_space(&mut functions, &authority, |_, event| {
                fake.access(event)
            })
            .expect("memory decode enabled");
            super::admit_bus_master(&mut functions, &authority, &mut state, |_, event| {
                fake.access(event)
            })
            .expect("isolated bus mastering admitted");

            fake.calls = 0;
            fake.trace = [None; COMMAND_TRACE_CAPACITY];
            fake.injected_read = (fail_at >= 3).then_some((2, 1 << 10));
            fake.fail_at = Some(fail_at);
            assert!(
                super::disable_bus_master_for_retirement(
                    &mut functions,
                    &authority,
                    &mut state,
                    |_, event| fake.access(event),
                )
                .is_err(),
                "access {fail_at} unexpectedly completed"
            );
            assert_eq!(state.before, original, "access {fail_at}");

            fake.injected_read = None;
            fake.fail_at = None;
            super::disable_bus_master_for_retirement(
                &mut functions,
                &authority,
                &mut state,
                |_, event| fake.access(event),
            )
            .expect("the same command owner retries retirement");
            assert_eq!(fake.command & command::BUS_MASTER, 0, "access {fail_at}");
            super::restore_command(&mut functions, &authority, state, |_, event| {
                fake.access(event)
            })
            .expect("safe command state remains exactly releasable");
            assert_eq!(fake.command, original & !command::BUS_MASTER);
            release_function(&mut functions, authority).expect("settled function releases");
        }
    }

    #[test]
    fn every_enable_error_describes_its_cause() {
        assert_eq!(
            EnableError::Access.to_string(),
            "configuration access failed"
        );
        assert_eq!(
            EnableError::Refused(0x00A5).to_string(),
            "the command register read back as 0x00a5"
        );
        assert_eq!(
            EnableError::Restore.to_string(),
            "the original command could not be verified"
        );
    }

    #[test]
    fn a_refused_bus_master_enable_restores_the_exact_command() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | OTHER;
        let wanted = original | command::MEMORY_SPACE | command::BUS_MASTER;
        let observed = wanted & !command::BUS_MASTER;
        let mut fake = FakeCommand::new(original, !command::BUS_MASTER);
        let mut leases = CommandLeases::<1>::new();

        assert_eq!(
            enable_bus_master(&mut leases, command_address(0), |_, event| fake
                .access(event))
            .map_err(|failure| failure.reason()),
            Err(EnableError::Refused(observed))
        );
        assert_eq!(fake.command, original);
        assert_eq!(
            fake.events(),
            &[
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(wanted)),
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(original)),
                Some(CommandAccess::Read),
            ]
        );
    }

    #[test]
    fn a_bus_master_enable_refuses_and_restores_an_unwritten_command_bit() {
        const OTHER: u16 = 1 << 9;
        let original = OTHER;
        let wanted = original | command::MEMORY_SPACE | command::BUS_MASTER;
        let observed = wanted | command::IO_SPACE;
        let mut fake = FakeCommand::new(original, u16::MAX);
        fake.injected_read = Some((2, command::IO_SPACE));
        let mut leases = CommandLeases::<1>::new();

        assert_eq!(
            enable_bus_master(&mut leases, command_address(0), |_, event| fake
                .access(event))
            .map_err(|failure| failure.reason()),
            Err(EnableError::Refused(observed))
        );
        assert_eq!(fake.command, original);
        assert_eq!(
            fake.events(),
            &[
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(wanted)),
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(original)),
                Some(CommandAccess::Read),
            ]
        );
    }

    #[test]
    fn every_failed_bus_master_access_restores_or_reports_uncertainty() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | OTHER;

        for fail_at in 0..=2 {
            let mut fake = FakeCommand::new(original, u16::MAX);
            fake.fail_at = Some(fail_at);
            let mut leases = CommandLeases::<1>::new();
            let failure = enable_bus_master(&mut leases, command_address(0), |_, event| {
                fake.access(event)
            })
            .expect_err("the injected access must refuse enablement");
            assert_eq!(failure.reason(), EnableError::Access, "access {fail_at}");
            assert_eq!(failure.into_state(), None, "access {fail_at}");
            assert_eq!(fake.command, original, "access {fail_at}");
        }

        let refuses_bus_master = !command::BUS_MASTER;
        let mut restore_write = FakeCommand::new(original, refuses_bus_master);
        restore_write.fail_at = Some(3);
        let mut write_leases = CommandLeases::<1>::new();
        assert_eq!(
            enable_bus_master(&mut write_leases, command_address(0), |_, event| {
                restore_write.access(event)
            })
            .expect_err("rollback uncertainty")
            .reason(),
            EnableError::Restore
        );
        assert_ne!(restore_write.command, original);

        let mut restore_read = FakeCommand::new(original, refuses_bus_master);
        restore_read.fail_at = Some(4);
        let mut read_leases = CommandLeases::<1>::new();
        assert_eq!(
            enable_bus_master(&mut read_leases, command_address(0), |_, event| {
                restore_read.access(event)
            })
            .expect_err("rollback uncertainty")
            .reason(),
            EnableError::Restore
        );
        assert_eq!(restore_read.command, original);
    }

    #[test]
    fn a_bus_master_lease_restores_the_exact_command_or_reports_uncertainty() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | OTHER;
        let mut restored = FakeCommand::new(original, u16::MAX);
        let mut restored_leases = CommandLeases::<1>::new();
        let state = enable_bus_master(&mut restored_leases, command_address(0), |_, event| {
            restored.access(event)
        })
        .expect("enabled");
        assert_eq!(
            restore_command(&mut restored_leases, state, |_, event| {
                restored.access(event)
            }),
            Ok(())
        );
        assert_eq!(restored.command, original);

        let mut refused = FakeCommand::new(original, u16::MAX);
        let mut refused_leases = CommandLeases::<1>::new();
        let state = enable_bus_master(&mut refused_leases, command_address(0), |_, event| {
            refused.access(event)
        })
        .expect("enabled");
        refused.writable = 0;
        let failure = restore_command(&mut refused_leases, state, |_, event| refused.access(event))
            .expect_err("a masked restore must retain its retry authority");
        assert_ne!(refused.command, original);

        refused.writable = u16::MAX;
        restore_command(&mut refused_leases, failure.into_state(), |_, event| {
            refused.access(event)
        })
        .expect("the unchanged state retries exactly");
        assert_eq!(refused.command, original);
    }

    #[test]
    fn an_address_puts_every_field_where_the_manual_says() {
        // Pairwise-distinct fields expose every accidental swap (D-136).
        let at = Address::new(0x12, 0x1A, 0x05, 0x3C).expect("a valid address");

        assert_eq!(at.bits() >> 31, 1, "the enable bit");
        assert_eq!(at.bus(), 0x12);
        assert_eq!(at.device(), 0x1A);
        assert_eq!(at.function(), 0x05);
        assert_eq!(at.bits() as u16 & 0xFF, 0x3C);
        assert_eq!(at.bits(), 0x8012_D53C);
    }

    #[test]
    fn every_address_round_trips_through_its_fields() {
        for bus in [0u8, 1, 0x7F, 0x80, 0xFE, 0xFF] {
            for device in 0..DEVICES {
                for function in 0..FUNCTIONS {
                    for offset in (0..HEADER_BYTES).step_by(4) {
                        let at =
                            Address::new(bus, device, function, offset).expect("all encodable");
                        assert_eq!(
                            (
                                at.bus(),
                                at.device(),
                                at.function(),
                                at.bits() as u16 & 0xFF,
                            ),
                            (bus, device, function, offset)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_offset_of_one_function_has_one_identity() {
        for start in (0..HEADER_BYTES).step_by(4) {
            let identity = Address::new(0x12, 0x1A, 5, start).expect("valid");
            for replacement in (0..HEADER_BYTES).step_by(4) {
                let alias = identity.at(replacement).expect("valid offset");
                assert_eq!(
                    (
                        alias.bus(),
                        alias.device(),
                        alias.function(),
                        alias.bits() as u16 & 0xFF
                    ),
                    (0x12, 0x1A, 5, replacement)
                );
                assert_eq!(alias.function_base(), identity.function_base());
            }
        }
        let identity = Address::new(0x12, 0x1A, 5, 0).expect("valid");
        assert_ne!(identity.at(header::CLASS), identity.at(header::BAR0));
    }

    #[test]
    fn function_claims_validate_headers_and_aliases_before_commit() {
        let at = Address::new(3, 4, 5, header::CLASS).expect("valid");
        let mut registry = FunctionClaims::<2>::new();
        assert_eq!(
            registry.claim(at, HeaderType::from_bits(2)),
            Err(FunctionClaimError::Header(2))
        );
        assert_eq!(registry.len(), 0);
        let authority = registry
            .claim(at, HeaderType::from_bits(0))
            .expect("a type-zero function is claimable");
        assert_eq!(authority.bar_registers(), BARS);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.len(), 1);
        assert!(registry.is_claimed(at.at(header::BAR0).expect("valid alias")));
        assert_eq!(
            registry.claim(
                at.at(header::VENDOR).expect("valid alias"),
                HeaderType::from_bits(1)
            ),
            Err(FunctionClaimError::Taken)
        );
        assert_eq!(registry.len(), 1, "duplicate refusal changed the holder");
        assert!(registry.holds(&authority));
    }

    #[test]
    fn function_authority_is_registry_bound_and_releases_exactly() {
        let first = Address::new(3, 4, 5, header::CLASS).expect("valid");
        let second = Address::new(3, 4, 6, header::VENDOR).expect("valid");
        let mut registry = FunctionClaims::<2>::new();
        let mut other = FunctionClaims::<2>::new();
        let first_authority = registry
            .claim(first, HeaderType::from_bits(0))
            .expect("first authority");
        let second_authority = registry
            .claim(second, HeaderType::from_bits(1))
            .expect("second authority");
        assert_eq!(second_authority.bar_registers(), BRIDGE_BARS);
        let foreign = other
            .claim(first, HeaderType::from_bits(0))
            .expect("the other registry has its own authority");
        let foreign = registry
            .release(foreign)
            .expect_err("a foreign token cannot release this registry");
        assert!(other.holds(&foreign));
        assert_eq!(registry.len(), 2, "a refused release changed the registry");

        registry
            .release(first_authority)
            .expect("the exact authority releases");
        assert!(!registry.is_claimed(first));
        assert!(registry.is_claimed(second));
        assert_eq!(registry.len(), 1);
        let replacement = registry
            .claim(first, HeaderType::from_bits(0))
            .expect("release made room for a new generation");
        assert_eq!(registry.len(), 2);
        registry.release(replacement).expect("replacement release");
        registry
            .release(second_authority)
            .expect("second exact release");
        other.release(foreign).expect("foreign issuer release");
    }

    #[test]
    fn configuration_cycles_require_the_issuing_function_registry() {
        let at = command_address(0);
        let mut issuer = FunctionClaims::<2>::new();
        let authority = issuer
            .claim(at, HeaderType::from_bits(0))
            .expect("issuing function authority");
        let other_authority = issuer
            .claim(command_address(1), HeaderType::from_bits(0))
            .expect("other function authority");
        let mut foreign = FunctionClaims::<1>::new();
        let mut foreign_accesses = 0;

        let refusal = super::enable_bus_master(&mut foreign, &authority, |_, _| {
            foreign_accesses += 1;
            Some(0)
        })
        .expect_err("another registry cannot spend the function authority");
        assert_eq!(refusal.reason(), EnableError::Stale);
        assert_eq!(refusal.into_state(), None);
        assert_eq!(foreign_accesses, 0, "a foreign token reached the device");
        assert_eq!(foreign.len(), 0);
        assert!(issuer.holds(&authority));

        let mut fake = FakeCommand::new(0, u16::MAX);
        let state =
            super::enable_bus_master(&mut issuer, &authority, |_, access| fake.access(access))
                .expect("the issuing registry reaches its function");
        let mut state = state;
        let issued_accesses = fake.calls;
        assert_eq!(
            super::disable_bus_master_for_retirement(
                &mut issuer,
                &other_authority,
                &mut state,
                |_, access| fake.access(access),
            ),
            Err(EnableError::Stale)
        );
        assert_eq!(
            fake.calls, issued_accesses,
            "a foreign registry reached retirement configuration"
        );
        super::disable_bus_master_for_retirement(
            &mut issuer,
            &authority,
            &mut state,
            |_, access| fake.access(access),
        )
        .expect("the issuing registry fences its function");
        super::restore_command(&mut issuer, &authority, state, |_, access| {
            fake.access(access)
        })
        .expect("the issuing registry restores its command");
        release_function(&mut issuer, other_authority).expect("other function release");
        release_function(&mut issuer, authority).expect("settled function authority releases");
    }

    #[test]
    fn public_memory_decode_accepts_only_its_function_authority() {
        let at = command_address(0);
        let mut issuer = FunctionClaims::<1>::new();
        let authority = issuer
            .claim(at, HeaderType::from_bits(0))
            .expect("issuing function authority");
        let mut foreign = FunctionClaims::<1>::new();
        let mut foreign_fake = FakeCommand::new(0, u16::MAX);

        let refusal = super::enable_memory_space(&mut foreign, &authority, |_, access| {
            foreign_fake.access(access)
        })
        .expect_err("another registry cannot enable memory decode");
        assert_eq!(refusal.reason(), EnableError::Stale);
        assert_eq!(refusal.into_state(), None);
        assert_eq!(foreign_fake.calls, 0, "a foreign token reached the device");

        let mut fake = FakeCommand::new(0, u16::MAX);
        let state =
            super::enable_memory_space(&mut issuer, &authority, |_, access| fake.access(access))
                .expect("the issuing registry enables memory decode");
        assert_eq!(fake.command, command::MEMORY_SPACE);
        super::restore_command(&mut issuer, &authority, state, |_, access| {
            fake.access(access)
        })
        .expect("memory decode restores exactly");
        release_function(&mut issuer, authority).expect("settled function authority releases");
    }

    #[test]
    fn public_bus_master_admission_changes_the_device_and_state() {
        let at = command_address(0);
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(at, HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeCommand::new(0, u16::MAX);
        let mut state =
            super::enable_memory_space(&mut functions, &authority, |_, access| fake.access(access))
                .expect("memory decode stage");

        super::admit_bus_master(&mut functions, &authority, &mut state, |_, access| {
            fake.access(access)
        })
        .expect("DMA admission");
        let admitted = command::MEMORY_SPACE | command::BUS_MASTER;
        assert_eq!(fake.command, admitted);
        assert_eq!(state.installed, admitted);
        super::restore_command(&mut functions, &authority, state, |_, access| {
            fake.access(access)
        })
        .expect("admitted command restores exactly");
        release_function(&mut functions, authority).expect("settled function authority releases");
    }

    #[test]
    fn public_command_commit_consumes_the_live_sublease() {
        let at = command_address(0);
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(at, HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeCommand::new(0, u16::MAX);
        let state =
            super::enable_bus_master(&mut functions, &authority, |_, access| fake.access(access))
                .expect("verified command transition");

        super::commit_command(&mut functions, &authority, state, |_, access| {
            fake.access(access)
        })
        .expect("the live command generation commits");
        assert_eq!(fake.command, command::MEMORY_SPACE | command::BUS_MASTER);
        release_function(&mut functions, authority).expect("commit released the command sublease");
    }

    #[test]
    fn public_probe_retry_restores_hardware_and_releases_its_sublease() {
        let at = command_address(0);
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(at, HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.fail_at = Some(30);
        let failure =
            super::probe_bars(&mut functions, &authority, |_, access| fake.access(access))
                .expect_err("one restoration read is unavailable");
        assert_eq!(failure.reason(), &ProbeError::Restore(30));
        let retry = failure
            .into_retry()
            .expect("uncertain restoration retains retry authority");

        fake.fail_at = None;
        super::retry_probe(&mut functions, &authority, retry, |_, access| {
            fake.access(access)
        })
        .expect("the issuing function restores its exact captured state");
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.status, original.status);
        assert_eq!(fake.bars, original.bars);
        release_function(&mut functions, authority).expect("retry released the probe sublease");
    }

    #[test]
    fn function_release_waits_for_every_command_and_probe_sublease() {
        let at = command_address(0);
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(at, HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeCommand::new(0, !command::BUS_MASTER);
        fake.fail_at = Some(4);
        let failure =
            super::enable_bus_master(&mut functions, &authority, |_, access| fake.access(access))
                .expect_err("rollback uncertainty keeps its command sublease");
        let state = failure.into_state().expect("uncertain command owner");

        let failure = release_function(&mut functions, authority)
            .expect_err("an uncertain command prevents function release");
        assert_eq!(failure.reason(), FunctionReleaseError::Active);
        let authority = failure.into_authority();
        assert!(functions.holds(&authority));
        assert_eq!(
            functions.claim(at, HeaderType::from_bits(0)),
            Err(FunctionClaimError::Taken)
        );

        fake.fail_at = None;
        fake.writable = u16::MAX;
        super::restore_command(&mut functions, &authority, state, |_, access| {
            fake.access(access)
        })
        .expect("the uncertain command restored");
        let probe = functions
            .commands
            .begin_probe(&authority)
            .expect("BAR probe sublease");
        let failure = release_function(&mut functions, authority)
            .expect_err("a live BAR probe prevents function release");
        assert_eq!(failure.reason(), FunctionReleaseError::Active);
        let authority = failure.into_authority();
        functions
            .commands
            .finish_probe(&authority, probe)
            .expect("the exact BAR sublease settles");
        release_function(&mut functions, authority).expect("settled function authority releases");
        assert_eq!(functions.len(), 0);
    }

    #[test]
    fn command_and_probe_subleases_cannot_cross_function_authorities() {
        let mut functions = FunctionClaims::<2>::new();
        let first = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("first function authority");
        let second = functions
            .claim(command_address(1), HeaderType::from_bits(0))
            .expect("second function authority");
        let mut fake = FakeCommand::new(0, u16::MAX);
        let state =
            super::enable_bus_master(&mut functions, &first, |_, access| fake.access(access))
                .expect("first command transition");
        let calls = fake.calls;
        let failure = super::commit_command(&mut functions, &second, state, |_, access| {
            fake.access(access)
        })
        .expect_err("another function cannot consume command authority");
        assert_eq!(failure.reason(), EnableError::Stale);
        assert_eq!(fake.calls, calls, "a foreign function reached the device");
        super::restore_command(&mut functions, &first, failure.into_state(), |_, access| {
            fake.access(access)
        })
        .expect("the issuing function retained its command owner");

        let probe = functions
            .commands
            .begin_probe(&first)
            .expect("first probe owner");
        let probe = functions
            .commands
            .finish_probe(&second, probe)
            .expect_err("another function cannot settle the probe owner");
        functions
            .commands
            .finish_probe(&first, probe)
            .expect("the issuing function settles the probe");
        release_function(&mut functions, first).expect("first release");
        release_function(&mut functions, second).expect("second release");
    }

    #[test]
    fn an_address_that_cannot_be_encoded_is_refused() {
        assert!(Address::new(0, DEVICES, 0, 0).is_none(), "device 32");
        assert!(Address::new(0, 0, FUNCTIONS, 0).is_none(), "function 8");
        assert!(
            Address::new(0, 0, 0, HEADER_BYTES).is_none(),
            "past the header"
        );
        for misaligned in [1u16, 2, 3, 0x0D, 0xFF] {
            assert!(
                Address::new(0, 0, 0, misaligned).is_none(),
                "offset {misaligned:#x} is not a dword boundary"
            );
        }

        let at = Address::new(0, 1, 0, 0).expect("valid");
        assert!(at.at(0x0D).is_none());
        assert!(at.at(HEADER_BYTES).is_none());
        assert_eq!(at.at(0x10).expect("valid").bits() as u16 & 0xFF, 0x10);
        assert_eq!(at.at(0x10).expect("valid").device(), 1, "the rest is kept");
    }

    #[test]
    fn the_class_word_is_read_as_four_separate_bytes() {
        // Distinct bytes prevent one field from standing in for another.
        let class = Class::from_bits(0x0C_03_30_A1);
        assert_eq!(class.class, 0x0C);
        assert_eq!(class.subclass, 0x03);
        assert_eq!(class.interface, 0x30);
        assert_eq!(class.revision, 0xA1);
    }

    #[test]
    fn nvme_is_all_three_bytes_and_its_neighbours_are_not() {
        assert!(Class::from_bits(0x01_08_02_00).is_nvme());

        for near in [
            0x01_08_01_00u32, // NVMHCI
            0x01_08_03_00,    // a prog-if this driver does not know
            0x01_06_02_00,    // SATA
            0x02_08_02_00,    // a network controller with the same tail
        ] {
            assert!(
                !Class::from_bits(near).is_nvme(),
                "{near:#010x} is not NVM Express"
            );
        }
    }

    #[test]
    fn a_bridge_is_recognised_and_nothing_else_is() {
        assert!(Class::from_bits(0x06_04_00_00).is_bridge());
        assert!(
            !Class::from_bits(0x06_00_00_00).is_bridge(),
            "a host bridge"
        );
        assert!(
            !Class::from_bits(0x06_01_00_00).is_bridge(),
            "an ISA bridge"
        );
        assert!(!Class::from_bits(0x01_08_02_00).is_bridge());
    }

    #[test]
    fn the_header_type_byte_is_a_layout_and_a_flag() {
        let plain = HeaderType::from_bits(0x00);
        assert!(!plain.is_multifunction());
        assert_eq!(plain.layout(), 0);
        assert!(!plain.is_bridge());

        let multi_bridge = HeaderType::from_bits(0x81);
        assert!(multi_bridge.is_multifunction());
        assert_eq!(multi_bridge.layout(), 1);
        assert!(multi_bridge.is_bridge());

        // The flag must not leak into the layout, which is what a missing mask
        // would do: `0x81 & 0x7F` is 1, `0x81` is 129.
        let multi_plain = HeaderType::from_bits(0x80);
        assert!(multi_plain.is_multifunction());
        assert_eq!(multi_plain.layout(), 0);
        assert!(!multi_plain.is_bridge());
    }

    #[test]
    fn a_base_address_register_decodes_to_its_window() {
        assert_eq!(Bar::decode(0, 0), Some(Bar::Absent));

        assert_eq!(Bar::decode(0xC001, 0), Some(Bar::Io { port: 0xC000 }));

        assert_eq!(
            Bar::decode(0xFEBF_0000, 0),
            Some(Bar::Memory {
                address: 0xFEBF_0000,
                wide: false,
                prefetchable: false,
            })
        );

        assert_eq!(
            Bar::decode(0xFEBF_000C, 0x0000_0001),
            Some(Bar::Memory {
                address: 0x1_FEBF_0000,
                wide: true,
                prefetchable: true,
            })
        );
        assert_eq!(
            Bar::decode(0xFEBF_0004, 0x0000_0001),
            Some(Bar::Memory {
                address: 0x1_FEBF_0000,
                wide: true,
                prefetchable: false,
            })
        );
    }

    #[test]
    fn every_memory_bar_type_encoding_has_one_exact_outcome() {
        const BASE: u32 = 0x0008_0000;
        const HIGH: u32 = 0x5A;

        for memory_type in 0..=3 {
            let decoded = Bar::decode(BASE | memory_type << 1, HIGH);
            match memory_type {
                0 => assert_eq!(
                    decoded,
                    Some(Bar::Memory {
                        address: u64::from(BASE),
                        wide: false,
                        prefetchable: false,
                    })
                ),
                1 | 3 => assert_eq!(decoded, None),
                2 => assert_eq!(
                    decoded,
                    Some(Bar::Memory {
                        address: u64::from(BASE) | (u64::from(HIGH) << 32),
                        wide: true,
                        prefetchable: false,
                    })
                ),
                _ => unreachable!("the complete two-bit domain was exhausted"),
            }
        }
    }

    #[test]
    fn a_memory_window_size_is_the_complement_of_the_probe() {
        for bits in 4..31u32 {
            let size = 1u32 << bits;
            let mask = !(size - 1);
            assert_eq!(
                memory_window_size(mask, 0, MemoryWidth::Bits32),
                Some(u64::from(size)),
                "a {size}-byte window"
            );
        }

        assert_eq!(
            memory_window_size(0, 0xFFFF_FFFF, MemoryWidth::Bits64),
            Some(0x1_0000_0000)
        );
        assert_eq!(
            memory_window_size(0xFFF0_0000, 0xFFFF_FFFF, MemoryWidth::Bits64),
            Some(0x10_0000)
        );
    }

    #[test]
    fn a_narrow_window_is_not_complemented_in_a_wide_word() {
        // The same mask must be complemented in the selected register width.
        assert_eq!(
            memory_window_size(0xFFFF_C000, 0, MemoryWidth::Bits32),
            Some(0x4000)
        );
        assert!(
            memory_window_size(0xFFFF_C000, 0, MemoryWidth::Bits64).unwrap() > u64::from(u32::MAX)
        );
    }

    #[test]
    fn an_io_window_size_is_a_port_count() {
        for bits in 2..16u32 {
            let size = 1u32 << bits;
            let mask = !(size - 1);
            assert_eq!(
                io_window_size(mask),
                u16::try_from(size).ok(),
                "a {size}-port window"
            );
        }
        assert_eq!(io_window_size(0xFFFF_FFE1), Some(32));
    }

    #[test]
    fn a_probe_that_decoded_nothing_is_refused() {
        assert_eq!(memory_window_size(0, 0, MemoryWidth::Bits32), None);
        assert_eq!(memory_window_size(0, 0, MemoryWidth::Bits64), None);
        assert_eq!(io_window_size(0), None);
        assert_eq!(
            memory_window_size(0x0000_000F, 0, MemoryWidth::Bits32),
            None
        );
        assert_eq!(io_window_size(0x0000_0003), None);
        assert_eq!(io_window_size(0xFFFF_0000), None);
    }

    #[test]
    fn probed_bars_return_their_complete_slot_array() {
        let first = Window {
            bar: Bar::Memory {
                address: 0x4000,
                wide: false,
                prefetchable: false,
            },
            size: 0x1000,
        };
        let mut windows = [None; BARS];
        windows[0] = Some(first);
        let probed = ProbedBars {
            function: None,
            windows,
        };

        assert_eq!(probed.into_windows(), windows);
    }

    #[test]
    fn a_bar_probe_refuses_only_register_counts_above_its_bound() {
        let mut accessed = false;
        assert_eq!(
            probe_bars(BARS + 1, |_| {
                accessed = true;
                Ok::<_, ()>(0)
            }),
            Err(ProbeError::Malformed)
        );
        assert!(!accessed);

        let mut fake = FakeConfig::device();
        fake.bars = [0; BARS];
        fake.masks = [0; BARS];
        assert!(probe_bars(1, |event| fake.access(event)).is_ok());
    }

    #[test]
    fn an_address_zero_memory_bar_is_discovered_from_its_mask() {
        let mut fake = FakeConfig::device();
        fake.bars = [0; BARS];
        fake.masks = [0xFFFF_F000, 0, 0, 0, 0, 0];

        let probed = probe_bars(1, |event| fake.access(event)).expect("valid BAR");
        assert_eq!(
            probed.into_windows()[0],
            Some(Window {
                bar: Bar::Memory {
                    address: 0,
                    wide: false,
                    prefetchable: false,
                },
                size: 0x1000,
            })
        );
    }

    #[test]
    fn an_io_address_bit_is_not_a_wide_memory_type() {
        let mut fake = FakeConfig::device();
        fake.bars = [0x0000_C005, 0, 0, 0, 0, 0];
        fake.masks = [0xFFFF_FFFD, 0, 0, 0, 0, 0];

        let probed = probe_bars(1, |event| fake.access(event)).expect("valid BAR");
        assert_eq!(
            probed.into_windows()[0],
            Some(Window {
                bar: Bar::Io { port: 0xC004 },
                size: 4,
            })
        );
    }

    #[test]
    fn unsupported_memory_bar_types_refuse_before_any_probe_write() {
        for memory_type in [1, 3] {
            let mut fake = FakeConfig::device();
            fake.bars = [0; BARS];
            fake.bars[0] = 0x0008_0000 | memory_type << 1;
            let original = fake.clone();

            assert_eq!(
                probe_bars(1, |event| fake.access(event)),
                Err(ProbeError::Malformed),
                "memory type {memory_type:02b}"
            );
            assert_eq!(
                fake.events(),
                [ProbeAccess::ReadCommand, ProbeAccess::ReadBar(0)].map(Some),
                "memory type {memory_type:02b}"
            );
            assert_eq!(fake.command, original.command);
            assert_eq!(fake.bars, original.bars);
        }
    }

    #[test]
    fn the_claimed_header_alone_sets_the_public_bar_probe_extent() {
        for (function, header, registers) in [(0, 0u8, BARS), (1, 1u8, BRIDGE_BARS)] {
            let mut functions = FunctionClaims::<1>::new();
            let authority = functions
                .claim(command_address(function), HeaderType::from_bits(header))
                .expect("supported header authority");
            let mut fake = FakeConfig::device();

            let staged = super::probe_bars(&mut functions, &authority, |at, event| {
                assert_eq!(at, authority.address());
                fake.access(event)
            })
            .expect("header-sized probe");
            let (_, state) = staged.into_parts();

            assert!(fake.events().iter().any(|event| {
                matches!(event, Some(ProbeAccess::ReadBar(index)) if *index == registers - 1)
            }));
            assert!(!fake.events().iter().any(|event| {
                matches!(event, Some(ProbeAccess::ReadBar(index) | ProbeAccess::WriteBar(index, _)) if *index >= registers)
            }));
            super::restore_command(&mut functions, &authority, state, |_, access| {
                fake_command_access(&mut fake, access)
            })
            .expect("the staged command restores");
            release_function(&mut functions, authority)
                .expect("a retired probe releases its function");
        }
    }

    #[test]
    fn inherited_bus_master_stays_off_until_the_staged_command_retires() {
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeConfig::device();
        let original = fake.command;
        assert_ne!(original & command::BUS_MASTER, 0);

        let staged = super::probe_bars(&mut functions, &authority, |_, event| fake.access(event))
            .expect("the BARs probe with inherited DMA disabled");
        let (_, state) = staged.into_parts();
        assert_eq!(fake.bar_writes_with_bus_master, 0);
        assert_eq!(fake.command, original & !PROBE_DISABLED);

        let failure = release_function(&mut functions, authority)
            .expect_err("the staged command keeps its function authority");
        assert_eq!(failure.reason(), FunctionReleaseError::Active);
        let authority = failure.into_authority();
        super::restore_command(&mut functions, &authority, state, |_, access| {
            fake_command_access(&mut fake, access)
        })
        .expect("retirement restores the inherited command exactly");
        assert_eq!(fake.command, original);
        release_function(&mut functions, authority)
            .expect("the function releases only after command retirement");
    }

    #[test]
    fn refused_bus_master_clear_releases_only_after_exact_rollback() {
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeConfig::device();
        let original = fake.command;
        fake.ignore_write_at = Some(7);

        let failure = super::probe_bars(&mut functions, &authority, |_, event| fake.access(event))
            .expect_err("the device ignored the DMA-disable write");
        assert_eq!(failure.reason(), &ProbeError::Refused(original));
        assert!(failure.into_retry().is_none());
        assert_eq!(fake.bar_writes_with_bus_master, 0);
        assert_eq!(fake.command, original);
        release_function(&mut functions, authority)
            .expect("verified rollback released the probe sublease");
    }

    #[test]
    fn a_bar_probe_is_one_restored_decode_off_transaction() {
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        let probed = probe_bars(BARS, |event| fake.access(event)).expect("valid device");

        assert_eq!(fake.command, original.command);
        assert_eq!(
            fake.status, original.status,
            "status is never echoed into RW1C bits"
        );
        assert_eq!(fake.bars, original.bars);
        assert_eq!(
            probed.into_windows()[0],
            Some(Window {
                bar: Bar::Memory {
                    address: 0x1_8000_0000,
                    wide: true,
                    prefetchable: false,
                },
                size: 0x4000,
            })
        );
        assert_eq!(
            probed.into_windows()[1],
            None,
            "the high half is not another window"
        );
        assert_eq!(
            probed.into_windows()[2],
            Some(Window {
                bar: Bar::Io { port: 0xC000 },
                size: 32,
            })
        );

        let expected = [
            ProbeAccess::ReadCommand,
            ProbeAccess::ReadBar(0),
            ProbeAccess::ReadBar(1),
            ProbeAccess::ReadBar(2),
            ProbeAccess::ReadBar(3),
            ProbeAccess::ReadBar(4),
            ProbeAccess::ReadBar(5),
            ProbeAccess::WriteCommand(0),
            ProbeAccess::ReadCommand,
            ProbeAccess::WriteBar(0, u32::MAX),
            ProbeAccess::WriteBar(1, u32::MAX),
            ProbeAccess::ReadBar(0),
            ProbeAccess::ReadBar(1),
            ProbeAccess::WriteBar(2, u32::MAX),
            ProbeAccess::ReadBar(2),
            ProbeAccess::WriteBar(3, u32::MAX),
            ProbeAccess::ReadBar(3),
            ProbeAccess::WriteBar(4, u32::MAX),
            ProbeAccess::ReadBar(4),
            ProbeAccess::WriteBar(5, u32::MAX),
            ProbeAccess::ReadBar(5),
            ProbeAccess::WriteCommand(0),
            ProbeAccess::ReadCommand,
            ProbeAccess::WriteBar(5, 0),
            ProbeAccess::WriteBar(4, 0),
            ProbeAccess::WriteBar(3, 0),
            ProbeAccess::WriteBar(2, 0x0000_C001),
            ProbeAccess::WriteBar(1, 1),
            ProbeAccess::WriteBar(0, 0x8000_0004),
            ProbeAccess::ReadBar(5),
            ProbeAccess::ReadBar(4),
            ProbeAccess::ReadBar(3),
            ProbeAccess::ReadBar(2),
            ProbeAccess::ReadBar(1),
            ProbeAccess::ReadBar(0),
            ProbeAccess::WriteCommand(
                command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER,
            ),
            ProbeAccess::ReadCommand,
        ];
        assert_eq!(fake.events(), expected.map(Some));
    }

    #[test]
    fn every_probe_access_failure_restores_the_device() {
        // Calls 7 through 20 are the decode-off write and probe cycles. The
        // events after them are rollback itself and have a separate proof.
        for fail_at in 7..=20 {
            let mut fake = FakeConfig::device();
            let original = fake.clone();
            fake.fail_at = Some(fail_at);

            assert_eq!(
                probe_bars(BARS, |event| fake.access(event)),
                Err(ProbeError::Access(fail_at)),
                "failure at cycle {fail_at}"
            );
            assert_eq!(fake.command, original.command, "command at cycle {fail_at}");
            assert_eq!(fake.status, original.status, "status at cycle {fail_at}");
            assert_eq!(fake.bars, original.bars, "BARs at cycle {fail_at}");
            assert_eq!(
                fake.events().last(),
                Some(&Some(ProbeAccess::ReadCommand)),
                "decode is restored last at cycle {fail_at}"
            );
        }
    }

    #[test]
    fn a_restore_failure_still_attempts_every_later_restore() {
        for fail_at in 23..=28 {
            let mut fake = FakeConfig::device();
            let original = fake.clone();
            fake.fail_at = Some(fail_at);

            probe_bars(BARS, |event| fake.access(event))
                .expect("an applied restore write is verified by the later reads");
            assert_eq!(fake.command, original.command, "command at cycle {fail_at}");
            assert_eq!(fake.bars, original.bars, "BARs at cycle {fail_at}");
        }
    }

    #[test]
    fn an_ignored_decode_off_write_is_refused_before_any_bar_access() {
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.ignore_write_at = Some(7);

        assert_eq!(
            probe_bars(BARS, |event| fake.access(event)),
            Err(ProbeError::Refused(original.command))
        );
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.bars, original.bars);
        assert!(
            !fake.events()[7..]
                .iter()
                .any(|event| matches!(event, Some(ProbeAccess::WriteBar(_, _))))
        );
    }

    #[test]
    fn a_restore_read_failure_keeps_decode_off() {
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.fail_at = Some(30);

        assert_eq!(
            probe_bars(BARS, |event| fake.access(event)),
            Err(ProbeError::Restore(30))
        );
        assert_eq!(
            fake.command,
            original.command & !PROBE_DISABLED,
            "decode became live before every BAR was proven"
        );
        assert_eq!(fake.status, original.status);
        assert_eq!(fake.bars, original.bars);
        assert_eq!(fake.events().last(), Some(&Some(ProbeAccess::ReadBar(4))));
    }

    #[test]
    fn an_uncertain_bar_restore_retains_exclusion_and_retries_exactly() {
        let supplied = command_address(0);
        let function = supplied.function_base();
        let authority = device_authority(supplied);
        let mut leases = CommandLeases::<1>::new();
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.fail_at = Some(30);

        let failure = super::probe_bars_with_leases(&mut leases, &authority, |at, event| {
            assert_eq!(at, function);
            fake.access(event)
        })
        .expect_err("one restoration read was unavailable");
        assert_eq!(failure.reason(), &ProbeError::Restore(30));
        let retry = failure
            .into_retry()
            .expect("uncertain hardware retains its function reservation");
        assert_eq!(leases.begin_probe(&authority), Err(EnableError::Leased));
        assert_eq!(fake.command, original.command & !PROBE_DISABLED);

        fake.fail_at = None;
        super::retry_probe_with_leases(&mut leases, &authority, retry, |at, event| {
            assert_eq!(at, function);
            fake.access(event)
        })
        .expect("the same owner restored every captured register");
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.status, original.status);
        assert_eq!(fake.bars, original.bars);

        let probe = leases
            .begin_probe(&authority)
            .expect("exact retry released the reservation");
        leases
            .finish_probe(&authority, probe)
            .expect("the new probe releases");
    }

    #[test]
    fn a_retry_reproves_decode_is_off_before_restoring_bars() {
        let supplied = command_address(0);
        let function = supplied.function_base();
        let authority = device_authority(supplied);
        let mut leases = CommandLeases::<1>::new();
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.fail_at = Some(30);

        let failure = super::probe_bars_with_leases(&mut leases, &authority, |at, event| {
            assert_eq!(at, function);
            fake.access(event)
        })
        .expect_err("one restoration read was unavailable");
        let retry = failure
            .into_retry()
            .expect("the uncertain restoration retains its owner");

        fake.fail_at = None;
        fake.command = original.command;
        let retry_start = fake.calls;
        fake.ignore_write_at = Some(retry_start);
        let failure =
            super::retry_probe_with_leases(&mut leases, &authority, retry, |at, event| {
                assert_eq!(at, function);
                fake.access(event)
            })
            .expect_err("an ignored decode-off write cannot expose the BAR restore");
        assert_eq!(failure.reason(), &ProbeError::RestoreChanged);
        let retry = failure
            .into_retry()
            .expect("the failed recheck retains the same owner");
        assert_eq!(
            &fake.events()[retry_start..],
            &[
                Some(ProbeAccess::WriteCommand(
                    original.command & !PROBE_DISABLED,
                )),
                Some(ProbeAccess::ReadCommand),
            ]
        );

        fake.ignore_write_at = None;
        super::retry_probe_with_leases(&mut leases, &authority, retry, |at, event| {
            assert_eq!(at, function);
            fake.access(event)
        })
        .expect("an exact retry restores the captured device state");
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.bars, original.bars);
    }

    #[test]
    fn ignored_bar_and_command_restore_writes_retain_retry_authority() {
        let supplied = command_address(0);
        let function = supplied.function_base();
        let authority = device_authority(supplied);
        let mut leases = CommandLeases::<1>::new();
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.ignore_write_at = Some(26);

        let failure = super::probe_bars_with_leases(&mut leases, &authority, |at, event| {
            assert_eq!(at, function);
            fake.access(event)
        })
        .expect_err("an ignored BAR restoration write cannot be accepted");
        assert_eq!(failure.reason(), &ProbeError::RestoreChanged);
        let retry = failure
            .into_retry()
            .expect("the uncertain BAR restoration retains its owner");
        assert_eq!(leases.begin_probe(&authority), Err(EnableError::Leased));
        assert_eq!(fake.command, original.command & !PROBE_DISABLED);

        fake.ignore_write_at = None;
        super::retry_probe_with_leases(&mut leases, &authority, retry, |at, event| {
            assert_eq!(at, function);
            fake.access(event)
        })
        .expect("the same token retries the ignored BAR write");
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.bars, original.bars);

        let staged = super::probe_bars_with_leases(&mut leases, &authority, |at, event| {
            assert_eq!(at, function);
            fake.access(event)
        })
        .expect("a second probe stages its command");
        let (_, state) = staged.into_parts();
        fake.ignore_write_at = Some(fake.calls + 1);
        let failure =
            super::restore_command_with_leases(&mut leases, &authority, state, |_, access| {
                fake_command_access(&mut fake, access)
            })
            .expect_err("an ignored command restoration write cannot be accepted");
        assert_eq!(failure.reason(), EnableError::Refused(0));
        assert_eq!(leases.begin_probe(&authority), Err(EnableError::Leased));

        fake.ignore_write_at = None;
        super::restore_command_with_leases(
            &mut leases,
            &authority,
            failure.into_state(),
            |_, access| fake_command_access(&mut fake, access),
        )
        .expect("the same command owner retries the ignored write");
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.bars, original.bars);
    }

    #[test]
    fn an_ignored_final_probe_command_restore_retains_retry_authority() {
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        let staged = probe_bars_unleased(BARS, |event| fake.access(event))
            .expect("a valid probe reaches its final command restore");
        fake.ignore_write_at = Some(fake.calls);

        let failure = restore_staged_probe(staged, &mut |event| fake.access(event))
            .expect_err("an ignored final command write cannot complete restoration");
        assert_eq!(failure.reason, ProbeError::RestoreChanged);
        let restore = failure
            .restore
            .expect("the failed readback retains restoration authority");
        assert_eq!(fake.command, original.command & !PROBE_DISABLED);

        fake.ignore_write_at = None;
        finish_probe_restoration(restore, &mut |event| fake.access(event))
            .expect("the retained authority retries the final command restore");
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.bars, original.bars);
    }

    #[test]
    fn a_wide_bar_in_the_final_slot_is_refused_before_mutation() {
        let mut fake = FakeConfig::device();
        fake.bars[0] = 0x0000_C001;
        fake.bars[1] = 0x0000_0004;
        let original = fake.clone();

        assert_eq!(
            probe_bars(BRIDGE_BARS, |event| fake.access(event)),
            Err(ProbeError::Malformed)
        );
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.status, original.status);
        assert_eq!(fake.bars, original.bars);
        assert!(fake.events().iter().flatten().all(|event| !matches!(
            event,
            ProbeAccess::WriteCommand(_) | ProbeAccess::WriteBar(_, _)
        )));
    }

    #[test]
    fn an_invalid_bar_mask_is_refused_after_restoration() {
        let mut fake = FakeConfig::device();
        fake.masks[2] = 0xFFFF_FFF5;
        let original = fake.clone();

        assert_eq!(
            probe_bars(BARS, |event| fake.access(event)),
            Err(ProbeError::InvalidWindow)
        );
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.status, original.status);
        assert_eq!(fake.bars, original.bars);
    }

    #[test]
    fn a_misaligned_bar_base_is_refused_after_restoration() {
        let mut fake = FakeConfig::device();
        fake.bars = [0x0000_1800, 0, 0, 0, 0, 0];
        fake.masks = [0xFFFF_F000, 0, 0, 0, 0, 0];
        let original = fake.clone();

        assert_eq!(
            probe_bars(1, |event| fake.access(event)),
            Err(ProbeError::InvalidWindow)
        );
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.status, original.status);
        assert_eq!(fake.bars, original.bars);
    }

    fn probe_claimed_bars<const N: usize>(
        functions: &mut FunctionClaims<N>,
        authority: &FunctionAuthority,
        fake: &mut FakeConfig,
    ) -> (ProbedBars, CommandState) {
        super::probe_bars(functions, authority, |at, event| {
            assert_eq!(at, authority.address());
            fake.access(event)
        })
        .expect("the claimed function has valid BARs")
        .into_parts()
    }

    #[test]
    fn every_error_text_names_its_condition() {
        for (text, expected) in [
            (FunctionClaimError::Taken.to_string(), "claimed"),
            (FunctionReleaseError::Active.to_string(), "sublease"),
            (BarAuthorityError::Busy.to_string(), "mapping authority"),
            (BarIdentityError::Changed.to_string(), "changed"),
            (
                BarEnableError::Identity(BarIdentityError::Access).to_string(),
                "unreadable",
            ),
        ] {
            assert!(text.contains(expected), "{text:?}");
        }
    }

    #[test]
    fn a_bar_authority_names_its_index_and_refuses_foreign_or_forged_identities() {
        let mut functions = FunctionClaims::<2>::new();
        let first = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("first function");
        let second = functions
            .claim(command_address(1), HeaderType::from_bits(0))
            .expect("second function");
        let mut fake = FakeConfig::device();
        fake.bars = [0x8000_0000, 0x9000_0000, 0, 0, 0, 0];
        fake.masks = [0xFFFF_C000, 0xFFFF_F000, 0, 0, 0, 0];
        let (bars, mut state) = probe_claimed_bars(&mut functions, &first, &mut fake);
        let bar = bind_memory_bar(&mut functions, &first, &bars, 1).expect("BAR1 authority");
        assert_eq!(bar.index(), 1);

        let foreign = enable_memory_bar(&mut functions, &second, &bar, &mut state, |_, _| Some(0))
            .expect_err("a lease the authority was not bound to is refused on its own");
        assert_eq!(foreign, BarEnableError::Identity(BarIdentityError::Stale));

        let unmapped = BarAuthority {
            function: bar.function,
            index: 2,
            window: bar.window,
        };
        let stale = enable_memory_bar(&mut functions, &first, &unmapped, &mut state, |_, _| {
            Some(0)
        })
        .expect_err("an index the registry never mapped is stale");
        assert_eq!(stale, BarEnableError::Identity(BarIdentityError::Stale));
        let refused = release_memory_bar(&mut functions, unmapped)
            .expect_err("an index the registry never mapped cannot be released");
        assert_eq!(refused.to_string(), BarAuthorityError::Stale.to_string());

        release_memory_bar(&mut functions, bar).expect("the real authority returns");
        let held = release_function(&mut functions, first)
            .expect_err("a function with a live probe sublease stays claimed");
        assert!(held.to_string().contains("sublease"), "{held}");
        release_function(&mut functions, second).expect("an idle function releases");
    }

    #[test]
    fn enabling_memory_decode_keeps_a_firmware_enabled_bit_and_refuses_a_retired_phase() {
        let mut functions = FunctionClaims::<1>::new();
        let function = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeConfig::device();
        let (bars, mut state) = probe_claimed_bars(&mut functions, &function, &mut fake);
        let bar = bind_memory_bar(&mut functions, &function, &bars, 0).expect("BAR0 authority");

        // The second admission finds decode already enabled and must keep it.
        for admission in 0..2 {
            let _ = enable_memory_bar(&mut functions, &function, &bar, &mut state, |_, event| {
                match event {
                    BarEnableAccess::ReadBar(index) => Some(fake.bars[index]),
                    BarEnableAccess::Command(access) => fake_command_access(&mut fake, access),
                }
            });
            assert_ne!(
                fake.command & command::MEMORY_SPACE,
                0,
                "admission {admission} keeps decode"
            );
            assert_eq!(
                fake.command & command::BUS_MASTER,
                0,
                "admission {admission}"
            );
        }

        state.phase = CommandPhase::Restoring(fake.command);
        let stale =
            enable_memory_bar(
                &mut functions,
                &function,
                &bar,
                &mut state,
                |_, event| match event {
                    BarEnableAccess::ReadBar(index) => Some(fake.bars[index]),
                    BarEnableAccess::Command(access) => fake_command_access(&mut fake, access),
                },
            )
            .expect_err("a command transaction outside its installed phase is stale");
        assert_eq!(stale, BarEnableError::Command(EnableError::Stale));
    }

    #[test]
    fn a_command_readback_that_differs_after_restoration_is_reported() {
        fn claimed() -> (FunctionClaims<1>, FunctionAuthority) {
            let mut functions = FunctionClaims::<1>::new();
            let authority = functions
                .claim(command_address(0), HeaderType::from_bits(0))
                .expect("function authority");
            (functions, authority)
        }
        fn position(fake: &FakeConfig, want: impl Fn(&ProbeAccess) -> bool, last: bool) -> usize {
            let events = fake.events();
            let matches = |event: &Option<ProbeAccess>| event.as_ref().is_some_and(&want);
            let found = if last {
                events.iter().rposition(matches)
            } else {
                events.iter().position(matches)
            };
            found.expect("the rehearsal performed the access")
        }

        let (mut rehearsal, rehearsed) = claimed();
        let mut clean = FakeConfig::device();
        let _ = probe_claimed_bars(&mut rehearsal, &rehearsed, &mut clean);
        let first_bar_write = position(
            &clean,
            |event| matches!(event, ProbeAccess::WriteBar(..)),
            false,
        );

        let (mut rehearsal, rehearsed) = claimed();
        let mut failing = FakeConfig::device();
        failing.fail_at = Some(first_bar_write);
        let _ = super::probe_bars(&mut rehearsal, &rehearsed, |_, event| failing.access(event))
            .expect_err("the refused BAR write fails the probe");
        let restore_write = position(
            &failing,
            |event| matches!(event, ProbeAccess::WriteCommand(_)),
            true,
        );

        let (mut functions, authority) = claimed();
        let mut fake = FakeConfig::device();
        fake.fail_at = Some(first_bar_write);
        fake.ignore_write_at = Some(restore_write);
        let failure = super::probe_bars(&mut functions, &authority, |_, event| fake.access(event))
            .expect_err("the device ignored the command restore after a failed probe");
        assert_eq!(failure.reason(), &ProbeError::RestoreChanged);
    }

    #[test]
    fn a_bar_mapping_authority_is_linear_and_bound_to_its_probe_function() {
        let mut functions = FunctionClaims::<2>::new();
        let first = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("first function");
        let second = functions
            .claim(command_address(1), HeaderType::from_bits(0))
            .expect("second function");
        let mut fake = FakeConfig::device();
        let (bars, state) = probe_claimed_bars(&mut functions, &first, &mut fake);

        assert_eq!(
            bind_memory_bar(&mut functions, &second, &bars, 0),
            Err(BarAuthorityError::Stale)
        );
        assert_eq!(
            bind_memory_bar(&mut functions, &first, &bars, 2),
            Err(BarAuthorityError::NotMemory)
        );
        let bar = bind_memory_bar(&mut functions, &first, &bars, 0)
            .expect("the issuing function binds its memory BAR");
        assert_eq!(bar.index(), 0);
        assert_eq!(bar.window(), bars.window(0).expect("BAR0"));
        assert_eq!(
            bind_memory_bar(&mut functions, &first, &bars, 0),
            Err(BarAuthorityError::Busy)
        );

        let failure = release_function(&mut functions, first)
            .expect_err("a mapped BAR keeps its function claimed");
        assert_eq!(failure.reason(), FunctionReleaseError::Active);
        let first = failure.into_authority();
        release_memory_bar(&mut functions, bar).expect("exact BAR authority returns");
        super::restore_command(&mut functions, &first, state, |_, access| {
            fake_command_access(&mut fake, access)
        })
        .expect("the staged probe restores");
        release_function(&mut functions, first).expect("the settled first function releases");
        release_function(&mut functions, second).expect("the untouched second function releases");
    }

    #[test]
    fn changed_or_foreign_bar_identity_performs_zero_command_writes() {
        let mut functions = FunctionClaims::<2>::new();
        let first = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("first function");
        let second = functions
            .claim(command_address(1), HeaderType::from_bits(0))
            .expect("second function");
        let mut fake = FakeConfig::device();
        let (bars, mut state) = probe_claimed_bars(&mut functions, &first, &mut fake);
        let bar = bind_memory_bar(&mut functions, &first, &bars, 0).expect("BAR0 authority");

        let mut foreign_accesses = 0;
        let foreign = enable_memory_bar(&mut functions, &second, &bar, &mut state, |_, _| {
            foreign_accesses += 1;
            Some(0)
        })
        .expect_err("another function cannot spend the BAR authority");
        assert_eq!(foreign, BarEnableError::Identity(BarIdentityError::Stale));
        assert_eq!(foreign_accesses, 0);

        let mut command_writes = 0;
        let changed =
            enable_memory_bar(
                &mut functions,
                &first,
                &bar,
                &mut state,
                |_, event| match event {
                    BarEnableAccess::ReadBar(0) => Some(fake.bars[0] + 0x4000),
                    BarEnableAccess::ReadBar(1) => Some(fake.bars[1]),
                    BarEnableAccess::ReadBar(_) => None,
                    BarEnableAccess::Command(CommandAccess::Write(_)) => {
                        command_writes += 1;
                        Some(0)
                    }
                    BarEnableAccess::Command(CommandAccess::Read) => Some(u32::from(fake.command)),
                },
            )
            .expect_err("a changed BAR cannot enable decode");
        assert_eq!(changed, BarEnableError::Identity(BarIdentityError::Changed));
        assert_eq!(command_writes, 0);

        let inaccessible = enable_memory_bar(
            &mut functions,
            &first,
            &bar,
            &mut state,
            |_, event| match event {
                BarEnableAccess::ReadBar(0) => Some(fake.bars[0]),
                BarEnableAccess::ReadBar(1) => None,
                BarEnableAccess::ReadBar(_) => None,
                BarEnableAccess::Command(CommandAccess::Write(_)) => {
                    command_writes += 1;
                    Some(0)
                }
                BarEnableAccess::Command(CommandAccess::Read) => Some(u32::from(fake.command)),
            },
        )
        .expect_err("an incomplete wide identity cannot enable decode");
        assert_eq!(
            inaccessible,
            BarEnableError::Identity(BarIdentityError::Access)
        );
        assert_eq!(command_writes, 0);

        release_memory_bar(&mut functions, bar).expect("BAR authority returns");
        super::restore_command(&mut functions, &first, state, |_, access| {
            fake_command_access(&mut fake, access)
        })
        .expect("the staged probe restores");
        release_function(&mut functions, first).expect("first function releases");
        release_function(&mut functions, second).expect("second function releases");
    }

    #[test]
    fn every_type_only_bar_change_performs_zero_command_writes() {
        let mut functions = FunctionClaims::<1>::new();
        let function = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function");
        let mut fake = FakeConfig::device();
        fake.bars = [0x8000_0000, 0, 0, 0, 0, 0];
        fake.masks = [0xFFFF_C000, 0, 0, 0, 0, 0];
        let (bars, mut state) = probe_claimed_bars(&mut functions, &function, &mut fake);
        let bar = bind_memory_bar(&mut functions, &function, &bars, 0).expect("BAR0 authority");

        for memory_type in 1..=3 {
            let mut command_writes = 0;
            let changed =
                enable_memory_bar(&mut functions, &function, &bar, &mut state, |_, event| {
                    match event {
                        BarEnableAccess::ReadBar(0) => Some(fake.bars[0] | memory_type << 1),
                        BarEnableAccess::ReadBar(_) => None,
                        BarEnableAccess::Command(CommandAccess::Write(_)) => {
                            command_writes += 1;
                            Some(0)
                        }
                        BarEnableAccess::Command(CommandAccess::Read) => {
                            Some(u32::from(fake.command))
                        }
                    }
                })
                .expect_err("a type-only BAR change cannot enable decode");
            assert_eq!(
                changed,
                BarEnableError::Identity(BarIdentityError::Changed),
                "memory type {memory_type:02b}"
            );
            assert_eq!(command_writes, 0, "memory type {memory_type:02b}");
        }

        release_memory_bar(&mut functions, bar).expect("BAR authority returns");
        super::restore_command(&mut functions, &function, state, |_, access| {
            fake_command_access(&mut fake, access)
        })
        .expect("the staged probe restores");
        release_function(&mut functions, function).expect("function releases");
    }

    #[test]
    fn exact_wide_bar_reads_precede_the_first_decode_write() {
        let mut functions = FunctionClaims::<1>::new();
        let function = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function");
        let mut fake = FakeConfig::device();
        let (bars, mut state) = probe_claimed_bars(&mut functions, &function, &mut fake);
        let bar = bind_memory_bar(&mut functions, &function, &bars, 0).expect("BAR0 authority");
        let mut command = FakeCommand::new(0, u16::MAX);
        let mut trace = Vec::new();

        enable_memory_bar(&mut functions, &function, &bar, &mut state, |at, event| {
            trace.push(event);
            match event {
                BarEnableAccess::ReadBar(index) => {
                    assert_eq!(
                        at,
                        function
                            .address()
                            .at(header::BAR0 + index as u16 * 4)
                            .expect("BAR address")
                    );
                    fake.bars.get(index).copied()
                }
                BarEnableAccess::Command(access) => {
                    assert_eq!(
                        at,
                        function
                            .address()
                            .at(header::COMMAND)
                            .expect("command address")
                    );
                    command.access(access)
                }
            }
        })
        .expect("the unchanged mapped BAR enables memory decode");

        assert_eq!(
            trace,
            [
                BarEnableAccess::ReadBar(0),
                BarEnableAccess::ReadBar(1),
                BarEnableAccess::Command(CommandAccess::Read),
                BarEnableAccess::Command(CommandAccess::Write(command::MEMORY_SPACE)),
                BarEnableAccess::Command(CommandAccess::Read),
            ]
        );
        super::restore_command(&mut functions, &function, state, |_, access| {
            command.access(access)
        })
        .expect("the command lease restores");
        release_memory_bar(&mut functions, bar).expect("BAR authority returns");
        release_function(&mut functions, function).expect("function releases");
    }

    #[test]
    fn mapped_bus_master_admission_requires_its_bar_and_command_owner() {
        let mut functions = FunctionClaims::<2>::new();
        let first = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("first function");
        let second = functions
            .claim(command_address(1), HeaderType::from_bits(0))
            .expect("second function");
        let mut first_config = FakeConfig::device();
        let (first_bars, mut state) = probe_claimed_bars(&mut functions, &first, &mut first_config);
        let first_bar =
            bind_memory_bar(&mut functions, &first, &first_bars, 0).expect("first BAR authority");
        let mut second_config = FakeConfig::device();
        let (second_bars, second_state) =
            probe_claimed_bars(&mut functions, &second, &mut second_config);
        let second_bar = bind_memory_bar(&mut functions, &second, &second_bars, 0)
            .expect("second BAR authority");
        let mut command = FakeCommand::new(0, u16::MAX);
        enable_memory_bar(
            &mut functions,
            &first,
            &first_bar,
            &mut state,
            |_, access| match access {
                BarEnableAccess::ReadBar(index) => first_config.bars.get(index).copied(),
                BarEnableAccess::Command(access) => command.access(access),
            },
        )
        .expect("first function memory decode");
        let calls = command.calls;

        assert_eq!(
            admit_mapped_bus_master(
                &mut functions,
                &second_bar,
                command_address(1),
                &mut state,
                |_, access| command.access(access)
            ),
            Err(EnableError::Stale)
        );
        assert_eq!(command.calls, calls, "a foreign BAR reached configuration");
        assert_eq!(
            admit_mapped_bus_master(
                &mut functions,
                &first_bar,
                command_address(1),
                &mut state,
                |_, access| command.access(access)
            ),
            Err(EnableError::ForeignDomain)
        );
        assert_eq!(
            command.calls, calls,
            "a domain for another function reached configuration"
        );

        admit_mapped_bus_master(
            &mut functions,
            &first_bar,
            command_address(0),
            &mut state,
            |_, access| command.access(access),
        )
        .expect("the mapped issuing BAR admits bus master behind its own domain");
        assert_eq!(command.command, command::MEMORY_SPACE | command::BUS_MASTER);
        super::restore_command(&mut functions, &first, state, |_, access| {
            command.access(access)
        })
        .expect("the issuing function restores its command");
        super::restore_command(&mut functions, &second, second_state, |_, access| {
            fake_command_access(&mut second_config, access)
        })
        .expect("the second staged probe restores");
        release_memory_bar(&mut functions, first_bar).expect("first BAR release");
        release_memory_bar(&mut functions, second_bar).expect("second BAR release");
        release_function(&mut functions, first).expect("first function release");
        release_function(&mut functions, second).expect("second function release");
    }

    #[test]
    fn an_absent_function_reads_as_all_ones() {
        assert!(is_absent(0xFFFF));
        assert!(!is_absent(0x8086), "Intel");
        assert!(!is_absent(0x1B36), "QEMU");
        assert!(!is_absent(0x15AD), "VMware");
        assert!(!is_absent(0x0000), "a vendor of zero is odd, not absent");
    }

    #[test]
    fn a_bridge_secondary_bus_is_read_from_its_own_byte() {
        let bits = 0x0044_3322u32 << 8 | 0x11;
        assert_eq!(secondary_bus(bits), 0x22);
    }

    fn topology_address(bus: u8, device: u8, function: u8) -> Address {
        Address::new(bus, device, function, 0).expect("fixture address")
    }

    fn bridge_class() -> Class {
        Class::from_bits(0x06_04_00_00)
    }

    fn endpoint_class() -> Class {
        Class::from_bits(0x01_08_02_00)
    }

    fn bridge_header() -> HeaderType {
        HeaderType::from_bits(1)
    }

    fn endpoint_header() -> HeaderType {
        HeaderType::from_bits(0)
    }

    fn bus_numbers(primary: u8, secondary: u8, subordinate: u8) -> u32 {
        u32::from(primary) | (u32::from(secondary) << 8) | (u32::from(subordinate) << 16)
    }

    #[test]
    fn a_sealed_topology_binds_presence_ranges_and_ancestry() {
        let root = topology_address(0, 0x1c, 0);
        let child = topology_address(0x20, 0, 0);
        let root_device = topology_address(0x20, 3, 0);
        let child_device = topology_address(0x30, 4, 1);
        let mut topology = PciTopology::new();
        topology
            .observe(
                root,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0, 0x20, 0x3f)),
            )
            .expect("root bridge");
        topology
            .observe(
                child,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0x20, 0x30, 0x35)),
            )
            .expect("child bridge");
        for at in [root_device, child_device] {
            topology
                .observe(at, endpoint_class(), endpoint_header(), None)
                .expect("endpoint");
        }

        topology.seal().expect("rooted topology");
        assert!(topology.is_sealed());
        assert!(topology.contains_function(child_device));
        assert!(!topology.contains_function(topology_address(0x30, 4, 0)));
        assert_eq!(
            topology.bridge_range(root),
            Some(BridgeRange {
                secondary: 0x20,
                subordinate: 0x3f,
            })
        );
        assert!(topology.hierarchy_contains(root, root_device));
        assert!(topology.hierarchy_contains(root, child_device));
        assert!(topology.hierarchy_contains(child, child_device));
        assert!(!topology.hierarchy_contains(child, root_device));
    }

    #[test]
    fn duplicate_unrooted_and_contradictory_functions_refuse_topology() {
        let first = topology_address(0, 0x1c, 0);
        let second = topology_address(0, 0x1d, 0);
        let mut duplicate = PciTopology::new();
        duplicate
            .observe(
                first,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0, 0x20, 0x20)),
            )
            .expect("first parent");
        assert!(matches!(
            duplicate.observe(
                second,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0, 0x20, 0x20)),
            ),
            Err(TopologyError::DuplicateParent { bus: 0x20, .. })
        ));

        let mut unrooted = PciTopology::new();
        let endpoint = topology_address(0x40, 2, 0);
        unrooted
            .observe(endpoint, endpoint_class(), endpoint_header(), None)
            .expect("unrooted observation");
        assert_eq!(
            unrooted.seal(),
            Err(TopologyError::UnrootedFunction { at: endpoint })
        );

        let mut contradictory = PciTopology::new();
        assert_eq!(
            contradictory.observe(first, bridge_class(), endpoint_header(), None),
            Err(TopologyError::BridgeIdentity { at: first })
        );
    }

    #[test]
    fn unreadable_and_invalid_bridge_bus_numbers_refuse_topology() {
        let bridge = topology_address(0x20, 0x1c, 0);
        for (numbers, expected) in [
            (None, TopologyError::MissingBusNumbers { at: bridge }),
            (
                Some(bus_numbers(0x21, 0x30, 0x3f)),
                TopologyError::InvalidRange {
                    at: bridge,
                    primary: 0x21,
                    secondary: 0x30,
                    subordinate: 0x3f,
                },
            ),
            (
                Some(bus_numbers(0x20, 0x20, 0x3f)),
                TopologyError::InvalidRange {
                    at: bridge,
                    primary: 0x20,
                    secondary: 0x20,
                    subordinate: 0x3f,
                },
            ),
            (
                Some(bus_numbers(0x20, 0x30, 0x2f)),
                TopologyError::InvalidRange {
                    at: bridge,
                    primary: 0x20,
                    secondary: 0x30,
                    subordinate: 0x2f,
                },
            ),
        ] {
            assert_eq!(
                PciTopology::new().observe(bridge, bridge_class(), bridge_header(), numbers),
                Err(expected)
            );
        }
    }

    #[test]
    fn child_escape_and_unrelated_overlap_refuse_topology() {
        let outer = topology_address(0, 0x1c, 0);
        let child = topology_address(0x20, 0, 0);
        let mut escaped = PciTopology::new();
        escaped
            .observe(
                outer,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0, 0x20, 0x2f)),
            )
            .expect("outer bridge");
        escaped
            .observe(
                child,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0x20, 0x30, 0x40)),
            )
            .expect("locally forward child");
        assert_eq!(
            escaped.seal(),
            Err(TopologyError::ChildEscapesParent {
                child,
                parent: outer,
            })
        );

        let sibling = topology_address(0, 0x1d, 0);
        let mut overlap = PciTopology::new();
        overlap
            .observe(
                outer,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0, 0x20, 0x3f)),
            )
            .expect("first root bridge");
        overlap
            .observe(
                sibling,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0, 0x30, 0x35)),
            )
            .expect("second root bridge");
        assert_eq!(
            overlap.seal(),
            Err(TopologyError::UnrelatedOverlap {
                first: outer,
                second: sibling,
            })
        );
    }

    #[test]
    fn a_bridge_subordinate_bus_is_read_from_its_own_byte() {
        // Primary, secondary, subordinate and latency timer, each distinct.
        let bits = 0x4433_2211u32;
        assert_eq!(subordinate_bus(bits), 0x33);
    }

    #[test]
    fn a_bridge_primary_bus_is_read_from_its_own_byte() {
        // Primary, secondary, subordinate and latency timer, each distinct.
        let bits = 0x4433_2211;
        assert_eq!(primary_bus(bits), 0x11);
    }
}
