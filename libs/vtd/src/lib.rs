//! Intel VT-d DMA and interrupt-remapping register, table and invalidation encoding.
//!
//! ```
//! use vtd::{AddressWidth, Capability, ContextEntry, RootEntry, enable_translation, gcmd, second_level};
//!
//! // SAGAW bit 2 advertises 4-level paging and MGAW field 47 a 48-bit width.
//! let capability = Capability::from_bits((1 << (8 + 2)) | (47 << 16));
//! let width = AddressWidth::choose(capability).expect("4-level fits within MGAW");
//! assert_eq!(width.levels(), 4);
//! assert!(width.covers(0xFFFF_FFFF_FFFF) && !width.covers(1 << 48));
//!
//! let root = RootEntry::try_present(0x1000).expect("page aligned");
//! assert_eq!(root.context_table_address(), 0x1000);
//! let context = ContextEntry::try_present(0x2000, width, 7).expect("page aligned");
//! assert_eq!(context.domain_id(), 7);
//! assert_eq!(second_level::pte(0x3000, true, true), 0x3000 | 0b11);
//!
//! assert_eq!(enable_translation(0) & gcmd::TE, gcmd::TE);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

/// Physical-address field of a table entry, bits 63:12; the low twelve carry flags.
const ADDRESS_MASK: u64 = 0xFFFF_FFFF_FFFF_F000;

/// Version register (32-bit).
pub const VERSION: u64 = 0x00;
/// Capability register (64-bit).
pub const CAP: u64 = 0x08;
/// Extended Capability register (64-bit).
pub const ECAP: u64 = 0x10;
/// Global Command register (32-bit).
pub const GCMD: u64 = 0x18;
/// Global Status register (32-bit).
pub const GSTS: u64 = 0x1C;
/// Root Table Address register (64-bit).
pub const RTADDR: u64 = 0x20;
/// Context Command register (64-bit).
pub const CCMD: u64 = 0x28;
/// Fault Status register (32-bit).
pub const FSTS: u64 = 0x34;
/// Fault Event Control register (32-bit).
pub const FECTL: u64 = 0x38;
/// Invalidation Queue Head register (64-bit).
pub const IQH: u64 = 0x80;
/// Invalidation Queue Tail register (64-bit).
pub const IQT: u64 = 0x88;
/// Invalidation Queue Address register (64-bit).
pub const IQA: u64 = 0x90;
/// Interrupt Remapping Table Address register (64-bit).
pub const IRTA: u64 = 0xB8;

/// The architecture version implemented by a remapping unit.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Version(u32);

impl Version {
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Major architecture version, bits 7:4.
    pub const fn major(self) -> u8 {
        ((self.0 >> 4) & 0xF) as u8
    }

    /// Minor architecture version, bits 3:0.
    pub const fn minor(self) -> u8 {
        (self.0 & 0xF) as u8
    }

    /// Whether register invalidation runs: majors six and up require queued, zero is malformed.
    pub const fn register_invalidation_supported(self) -> bool {
        matches!(self.major(), 1..=5)
    }
}

/// Fault Event Control bits; events bypass interrupt remapping, so the owner keeps them masked.
pub mod fault_event_control {
    /// A fault event is pending while delivery is masked (IP, read-only).
    pub const PENDING: u32 = 1 << 30;
    /// Prohibit the unit from generating its fault-event interrupt (IM).
    pub const MASK: u32 = 1 << 31;

    /// Whether hardware still reports fault-event delivery masked.
    pub const fn masked(control: u32) -> bool {
        control & MASK != 0
    }
}

/// The root-table pointer in bits 63:12 of RTADDR; bits 11:10 select the mode, left at legacy.
pub const fn root_table_address(rtaddr: u64) -> u64 {
    rtaddr & ADDRESS_MASK
}

/// The Capability register.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Capability(u64);

impl Capability {
    /// Number of domains field, bits 2:0.
    const ND_MASK: u64 = 0x7;
    /// Required write-buffer flushing, bit 4.
    const RWBF: u64 = 1 << 4;
    /// Caching mode, bit 7.
    const CM: u64 = 1 << 7;
    /// Supported adjusted guest address widths, bits 12:8.
    const SAGAW_SHIFT: u32 = 8;
    const SAGAW_MASK: u64 = 0x1F;
    /// Maximum guest address width, bits 21:16.
    const MGAW_SHIFT: u32 = 16;
    const MGAW_MASK: u64 = 0x3F;
    /// Fault-recording register offset, bits 33:24.
    const FRO_SHIFT: u32 = 24;
    const FRO_MASK: u64 = 0x3FF;
    /// Number of fault-recording registers, bits 47:40.
    const NFR_SHIFT: u32 = 40;
    const NFR_MASK: u64 = 0xFF;
    /// Write draining support, bit 54.
    const DWD: u64 = 1 << 54;
    /// Read draining support, bit 55.
    const DRD: u64 = 1 << 55;
    /// Enhanced Set Interrupt Remap Table Pointer support, bit 62.
    const ESIRTPS: u64 = 1 << 62;

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Domains the unit supports: `2^(4 + 2 * ND)`.
    pub const fn number_of_domains(self) -> u32 {
        let nd = (self.0 & Self::ND_MASK) as u32;
        1u32 << (4 + 2 * nd)
    }

    /// Whether the unit requires an explicit write-buffer flush (RWBF).
    pub const fn requires_write_buffer_flush(self) -> bool {
        (self.0 & Self::RWBF) != 0
    }

    /// Whether the unit caches not-present entries (CM), so every such change needs invalidation.
    pub const fn caching_mode(self) -> bool {
        (self.0 & Self::CM) != 0
    }

    /// Whether SAGAW bit `levels - 2` is set: bit 0 is 2-level, bit 3 is 5-level.
    pub const fn sagaw_supports(self, levels: u8) -> bool {
        if levels < 2 {
            return false;
        }
        let bit = levels - 2;
        if bit >= 5 {
            return false;
        }
        let sagaw = (self.0 >> Self::SAGAW_SHIFT) & Self::SAGAW_MASK;
        ((sagaw >> bit) & 1) != 0
    }

    /// Maximum guest address width in bits: the MGAW field plus one.
    pub const fn max_guest_address_width(self) -> u8 {
        (((self.0 >> Self::MGAW_SHIFT) & Self::MGAW_MASK) as u8) + 1
    }

    /// Byte offset of the first fault-recording register: FRO times sixteen.
    pub const fn fault_recording_offset_bytes(self) -> u64 {
        ((self.0 >> Self::FRO_SHIFT) & Self::FRO_MASK) * 16
    }

    /// Fault-recording registers the unit has: the NFR field plus one.
    pub const fn number_of_fault_recording_registers(self) -> u16 {
        (((self.0 >> Self::NFR_SHIFT) & Self::NFR_MASK) as u16) + 1
    }

    /// Whether translation transitions drain in-flight DMA writes.
    pub const fn write_draining(self) -> bool {
        (self.0 & Self::DWD) != 0
    }

    /// Whether translation transitions drain in-flight DMA reads.
    pub const fn read_draining(self) -> bool {
        (self.0 & Self::DRD) != 0
    }

    /// Whether a completed translation transition drains both DMA directions.
    pub const fn dma_draining(self) -> bool {
        self.write_draining() && self.read_draining()
    }

    /// Whether SIRTP itself globally invalidates interrupt-remapping caches.
    pub const fn enhanced_set_interrupt_table_pointer(self) -> bool {
        (self.0 & Self::ESIRTPS) != 0
    }
}

/// The Extended Capability register.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ExtendedCapability(u64);

impl ExtendedCapability {
    /// Page-walk coherency, bit 0.
    const C: u64 = 1 << 0;
    /// Queued invalidation support, bit 1.
    const QI: u64 = 1 << 1;
    /// Interrupt remapping support, bit 3.
    const IR: u64 = 1 << 3;
    /// Extended interrupt mode support, bit 4.
    const EIM: u64 = 1 << 4;
    /// IOTLB register offset, bits 17:8.
    const IRO_SHIFT: u32 = 8;
    const IRO_MASK: u64 = 0x3FF;

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Whether table walks are coherent (C); when clear, each entry write needs a cache flush.
    pub const fn page_walk_coherent(self) -> bool {
        (self.0 & Self::C) != 0
    }

    /// Whether queued invalidation is supported (QI).
    pub const fn queued_invalidation(self) -> bool {
        (self.0 & Self::QI) != 0
    }

    /// Whether interrupt remapping is supported (IR).
    pub const fn interrupt_remapping(self) -> bool {
        (self.0 & Self::IR) != 0
    }

    /// Whether interrupt remapping can target full x2APIC identifiers (EIM).
    pub const fn extended_interrupt_mode(self) -> bool {
        (self.0 & Self::EIM) != 0
    }

    /// Byte offset of the IOTLB register block: IRO times sixteen.
    pub const fn iotlb_register_offset_bytes(self) -> u64 {
        ((self.0 >> Self::IRO_SHIFT) & Self::IRO_MASK) * 16
    }
}

/// Global Command bits, each written over a software-preserved shadow of the enable bits.
pub mod gcmd {
    /// Translation Enable, bit 31.
    pub const TE: u32 = 1 << 31;
    /// Set Root Table Pointer, bit 30.
    pub const SRTP: u32 = 1 << 30;
    /// Write Buffer Flush, bit 27.
    pub const WBF: u32 = 1 << 27;
    /// Queued Invalidation Enable, bit 26.
    pub const QIE: u32 = 1 << 26;
    /// Interrupt Remapping Enable, bit 25.
    pub const IRE: u32 = 1 << 25;
    /// Set Interrupt Remap Table Pointer, bit 24.
    pub const SIRTP: u32 = 1 << 24;
    /// Compatibility Format Interrupt, bit 23.
    pub const CFI: u32 = 1 << 23;
}

/// Global Status register bits.
pub mod gsts {
    /// Translation Enable Status, bit 31.
    pub const TES: u32 = 1 << 31;
    /// Root Table Pointer Status, bit 30.
    pub const RTPS: u32 = 1 << 30;
    /// Write Buffer Flush Status, bit 27.
    pub const WBFS: u32 = 1 << 27;
    /// Queued Invalidation Enable Status, bit 26.
    pub const QIES: u32 = 1 << 26;
    /// Interrupt Remapping Enable Status, bit 25.
    pub const IRES: u32 = 1 << 25;
    /// Interrupt Remap Table Pointer Status, bit 24.
    pub const IRTPS: u32 = 1 << 24;
    /// Compatibility Format Interrupt Status, bit 23.
    pub const CFIS: u32 = 1 << 23;
}

/// OR the Set-Root-Table-Pointer bit into a preserved command word.
pub const fn set_root_table_pointer(command: u32) -> u32 {
    command | gcmd::SRTP
}

/// OR the Translation-Enable bit into a preserved command word.
pub const fn enable_translation(command: u32) -> u32 {
    command | gcmd::TE
}

/// OR the Set-Interrupt-Remap-Table-Pointer bit into a command word.
pub const fn set_interrupt_table_pointer(command: u32) -> u32 {
    command | gcmd::SIRTP
}

/// A command word with remapping on and compatibility-format delivery off.
pub const fn enable_interrupt_remapping(command: u32) -> u32 {
    (command | gcmd::IRE) & !gcmd::CFI
}

/// Whether the root table pointer has been accepted (RTPS).
pub const fn root_table_pointer_set(status: u32) -> bool {
    (status & gsts::RTPS) != 0
}

/// Whether DMA translation is enabled (TES).
pub const fn translation_enabled(status: u32) -> bool {
    (status & gsts::TES) != 0
}

/// Whether the interrupt-remapping table pointer was accepted (IRTPS).
pub const fn interrupt_table_pointer_set(status: u32) -> bool {
    (status & gsts::IRTPS) != 0
}

/// Whether interrupt remapping is enabled (IRES).
pub const fn interrupt_remapping_enabled(status: u32) -> bool {
    (status & gsts::IRES) != 0
}

/// Whether compatibility-format interrupts bypass remapping (CFIS).
pub const fn compatibility_interrupts_enabled(status: u32) -> bool {
    (status & gsts::CFIS) != 0
}

/// Whether firmware left translation, queued invalidation and interrupt remapping all inactive.
///
/// Only from this state may an early owner program a new root, since a live SRTP or an
/// unfinished firmware quiescence protocol cannot be trusted.
pub const fn takeover_ready(status: u32) -> bool {
    status & (gsts::TES | gsts::QIES | gsts::IRES | gsts::CFIS) == 0
}

/// Address bits carried by IRTA and IQA.
const REMAPPING_ADDRESS_MASK: u64 = 0xFFFF_FFFF_FFFF_F000;

/// Encode a two-entry (size zero) interrupt-remapping table at `address`, with EIME set.
///
/// EIME is set because only x2APIC delivery is admitted, and extended mode blocks
/// compatibility-format requests independently of CFI.
pub const fn interrupt_table_address(address: u64) -> Option<u64> {
    if address & !REMAPPING_ADDRESS_MASK != 0 {
        return None;
    }
    Some((address & REMAPPING_ADDRESS_MASK) | (1 << 11))
}

/// Recover the table address from an IRTA register value.
pub const fn interrupt_table_base(value: u64) -> u64 {
    value & REMAPPING_ADDRESS_MASK
}

/// Whether IRTA selects extended interrupt mode (EIME).
pub const fn extended_interrupt_mode_enabled(value: u64) -> bool {
    value & (1 << 11) != 0
}

/// Whether IRTA selects exactly the two-entry table this kernel owns.
pub const fn interrupt_table_has_two_entries(value: u64) -> bool {
    value & 0xF == 0
}

/// Encode a one-page queue of 128-bit invalidation descriptors.
pub const fn invalidation_queue_address(address: u64) -> Option<u64> {
    if address & !REMAPPING_ADDRESS_MASK != 0 {
        return None;
    }
    Some(address & REMAPPING_ADDRESS_MASK)
}

/// A 128-bit queued-invalidation descriptor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InvalidationDescriptor {
    pub lo: u64,
    pub hi: u64,
}

impl InvalidationDescriptor {
    /// Globally invalidate the interrupt-entry cache.
    pub const fn global_interrupt_entry_cache() -> Self {
        Self { lo: 4, hi: 0 }
    }

    /// Wait for older descriptors, then write `data` to aligned `address`.
    pub const fn wait_status(address: u64, data: u32) -> Option<Self> {
        if address & 0b11 != 0 {
            return None;
        }
        Some(Self {
            lo: 5 | (1 << 5) | ((data as u64) << 32),
            hi: address,
        })
    }
}

/// A supported adjusted guest address width and its page-table depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddressWidth {
    /// 39-bit addresses, a 3-level second-level page table.
    Level3,
    /// 48-bit addresses, a 4-level second-level page table.
    Level4,
    /// 57-bit addresses, a 5-level second-level page table.
    Level5,
}

impl AddressWidth {
    pub const fn levels(self) -> u8 {
        match self {
            Self::Level3 => 3,
            Self::Level4 => 4,
            Self::Level5 => 5,
        }
    }

    pub const fn address_bits(self) -> u8 {
        match self {
            Self::Level3 => 39,
            Self::Level4 => 48,
            Self::Level5 => 57,
        }
    }

    /// Whether `address` can be named by a walk of this width; above it would alias the top index.
    pub const fn covers(self, address: u64) -> bool {
        address >> self.address_bits() == 0
    }

    /// The context-entry AW field encoding: 1, 2 or 3.
    pub const fn context_encoding(self) -> u64 {
        match self {
            Self::Level3 => 1,
            Self::Level4 => 2,
            Self::Level5 => 3,
        }
    }

    /// Whether `address` fits both this table and the unit's MGAW.
    pub const fn covers_on(self, capability: Capability, address: u64) -> bool {
        let bits = capability.max_guest_address_width();
        self.covers(address) && (bits == 64 || address >> bits == 0)
    }

    /// The widest modelled depth SAGAW supports; MGAW limits only [`Self::covers_on`].
    pub const fn choose(capability: Capability) -> Option<Self> {
        if capability.sagaw_supports(5) {
            return Some(Self::Level5);
        }
        if capability.sagaw_supports(4) {
            return Some(Self::Level4);
        }
        if capability.sagaw_supports(3) {
            return Some(Self::Level3);
        }
        None
    }

    /// The 9-bit index `iova` selects at `level`, where level 0 is the leaf table.
    pub const fn index(self, iova: u64, level: u8) -> u16 {
        let shift = 12 + 9 * (level as u32);
        ((iova >> shift) & 0x1FF) as u16
    }
}

/// A root-table entry, with [`lo`](Self::lo) at the entry's base and [`hi`](Self::hi) above it.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct RootEntry {
    /// Low qword: present bit and the context-table pointer.
    pub lo: u64,
    /// High qword: reserved, always zero.
    pub hi: u64,
}

impl RootEntry {
    /// Present, bit 0.
    const PRESENT: u64 = 1 << 0;

    /// A present root entry; a misaligned `context_table` silently loses its low twelve bits.
    #[must_use]
    pub const fn present(context_table: u64) -> Self {
        Self {
            lo: (context_table & ADDRESS_MASK) | Self::PRESENT,
            hi: 0,
        }
    }

    /// A present root entry, or `None` if `context_table` is not 4-KiB aligned.
    pub const fn try_present(context_table: u64) -> Option<Self> {
        if context_table & 0xFFF != 0 {
            return None;
        }
        Some(Self::present(context_table))
    }

    pub const fn is_present(self) -> bool {
        (self.lo & Self::PRESENT) != 0
    }

    pub const fn context_table_address(self) -> u64 {
        self.lo & ADDRESS_MASK
    }
}

/// A context-table entry: the low and high 64-bit words, little-endian.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ContextEntry {
    /// Low qword: present bit, translation type, and second-level root pointer.
    pub lo: u64,
    /// High qword: address-width encoding and domain identifier.
    pub hi: u64,
}

impl ContextEntry {
    /// Present, bit 0 of the low qword.
    const PRESENT: u64 = 1 << 0;
    /// Address-width field, bits 2:0 of the high qword.
    const AW_MASK: u64 = 0x7;
    /// Domain identifier, bits 23:8 of the high qword.
    const DID_SHIFT: u32 = 8;

    /// A present second-level-only entry; a misaligned root silently loses its low bits.
    #[must_use]
    pub const fn present(second_level_root: u64, width: AddressWidth, domain_id: u16) -> Self {
        Self {
            lo: (second_level_root & ADDRESS_MASK) | Self::PRESENT,
            hi: width.context_encoding() | ((domain_id as u64) << Self::DID_SHIFT),
        }
    }

    /// A present context entry, or `None` if `second_level_root` is not 4-KiB aligned.
    pub const fn try_present(
        second_level_root: u64,
        width: AddressWidth,
        domain_id: u16,
    ) -> Option<Self> {
        if second_level_root & 0xFFF != 0 {
            return None;
        }
        Some(Self::present(second_level_root, width, domain_id))
    }

    pub const fn is_present(self) -> bool {
        (self.lo & Self::PRESENT) != 0
    }

    /// The translation-type field, bits 3:2 of the low qword.
    pub const fn translation_type(self) -> u64 {
        (self.lo >> 2) & 0x3
    }

    /// The second-level page-table root physical address.
    pub const fn second_level_address(self) -> u64 {
        self.lo & ADDRESS_MASK
    }

    /// The address-width field encoding, bits 2:0 of the high qword.
    pub const fn address_width_encoding(self) -> u64 {
        self.hi & Self::AW_MASK
    }

    /// The domain identifier, bits 23:8 of the high qword.
    pub const fn domain_id(self) -> u16 {
        ((self.hi >> Self::DID_SHIFT) & 0xFFFF) as u16
    }
}

/// Second-level entries: read and write in bits 1:0, a 4-KiB-aligned address in bits 63:12.
pub mod second_level {
    use super::ADDRESS_MASK;

    /// Read permission, bit 0.
    const READ: u64 = 1 << 0;
    /// Write permission, bit 1.
    const WRITE: u64 = 1 << 1;

    pub const NOT_PRESENT: u64 = 0;

    /// A leaf entry mapping `physical`, whose low twelve bits are masked off.
    pub const fn pte(physical: u64, read: bool, write: bool) -> u64 {
        let mut bits = physical & ADDRESS_MASK;
        if read {
            bits |= READ;
        }
        if write {
            bits |= WRITE;
        }
        bits
    }

    /// A non-leaf entry with read and write set, so permission is decided at the leaves.
    pub const fn table_pointer(physical: u64) -> u64 {
        (physical & ADDRESS_MASK) | READ | WRITE
    }

    /// Whether an entry read back from a table is present with any permission.
    pub const fn is_present(entry: u64) -> bool {
        entry & (READ | WRITE) != 0
    }

    /// The physical address an entry names, whatever its permission bits.
    pub const fn address(entry: u64) -> u64 {
        entry & ADDRESS_MASK
    }
}

/// Context-cache invalidation through CCMD, which is reserved while queued invalidation is enabled.
pub mod context_command {
    /// Invalidate Context Cache, bit 63 (set by software, cleared when done).
    const ICC: u64 = 1 << 63;
    /// Context Invalidation Request Granularity, bits 62:61.
    const CIRG_SHIFT: u32 = 61;
    /// Global-granularity request (CIRG = 1).
    const GLOBAL: u64 = 1 << CIRG_SHIFT;
    /// Context Actual Invalidation Granularity, bits 60:59.
    const CAIG_SHIFT: u32 = 59;
    const CAIG_MASK: u64 = 0b11;
    const ACTUAL_GLOBAL: u64 = 0b01;

    pub const fn global() -> u64 {
        ICC | GLOBAL
    }

    /// Whether a read-back value shows the invalidation complete (ICC clear).
    pub const fn completed(value: u64) -> bool {
        (value & ICC) == 0
    }

    /// Whether a global request completed with hardware reporting a global invalidation (CAIG).
    pub const fn global_completed(value: u64) -> bool {
        completed(value) && ((value >> CAIG_SHIFT) & CAIG_MASK) == ACTUAL_GLOBAL
    }
}

/// IOTLB invalidation in the block [`ExtendedCapability::iotlb_register_offset_bytes`] locates.
pub mod iotlb_command {
    /// Offset of the IOTLB Invalidate register (IOTLB_REG) within the block.
    pub const COMMAND_REGISTER: u64 = 0x08;

    /// Invalidate IOTLB, bit 63 (set by software, cleared when done).
    const IVT: u64 = 1 << 63;
    /// IOTLB Invalidation Request Granularity, bits 61:60.
    const IIRG_SHIFT: u32 = 60;
    /// Global-granularity request (IIRG = 1).
    const GLOBAL: u64 = 1 << IIRG_SHIFT;
    /// Drain reads before completing the invalidation, bit 49.
    const DRAIN_READS: u64 = 1 << 49;
    /// Drain writes before completing the invalidation, bit 48.
    const DRAIN_WRITES: u64 = 1 << 48;
    /// IOTLB Actual Invalidation Granularity, bits 58:57.
    const IAIG_SHIFT: u32 = 57;
    const IAIG_MASK: u64 = 0b11;
    const ACTUAL_GLOBAL: u64 = 0b01;

    pub const fn global() -> u64 {
        IVT | GLOBAL
    }

    /// A global invalidation after older DMA drains; the caller must first require DRD and DWD.
    pub const fn global_with_drain() -> u64 {
        global() | DRAIN_READS | DRAIN_WRITES
    }

    /// Whether a read-back value shows the invalidation complete (IVT clear).
    pub const fn completed(value: u64) -> bool {
        (value & IVT) == 0
    }

    /// Whether a global request completed with hardware reporting a global invalidation (IAIG).
    pub const fn global_completed(value: u64) -> bool {
        completed(value) && ((value >> IAIG_SHIFT) & IAIG_MASK) == ACTUAL_GLOBAL
    }
}

/// The Fault Status register (FSTS).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct FaultStatus(u32);

impl FaultStatus {
    /// Primary fault overflow, bit 0.
    const PFO: u32 = 1 << 0;
    /// Primary pending fault, bit 1.
    const PPF: u32 = 1 << 1;
    /// Fault record index, bits 15:8.
    const FRI_SHIFT: u32 = 8;
    const FRI_MASK: u32 = 0xFF;

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// Whether a primary fault is pending (PPF).
    pub const fn primary_pending(self) -> bool {
        (self.0 & Self::PPF) != 0
    }

    /// Whether the log overflowed (PFO), which stops recording until it is acknowledged.
    pub const fn overflow(self) -> bool {
        (self.0 & Self::PFO) != 0
    }

    /// Index of the fault-recording register holding the first pending fault.
    pub const fn fault_record_index(self) -> u8 {
        ((self.0 >> Self::FRI_SHIFT) & Self::FRI_MASK) as u8
    }

    /// The write-1-to-clear PFO value that clears an overflow and writes nothing else.
    pub const fn acknowledge_overflow() -> u32 {
        Self::PFO
    }
}

/// A 16-byte fault-recording register: `lo` holds bits 63:0 and `hi` bits 127:64.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct FaultRecording {
    /// Low qword: bits 63:0 of the record, including the faulted address.
    pub lo: u64,
    /// High qword: bits 127:64, including the fault bit, reason and source id.
    pub hi: u64,
}

impl FaultRecording {
    /// Fault present, bit 127 (bit 63 of the high qword).
    const FAULT: u64 = 1 << 63;
    /// Request type, bit 126 (bit 62 of the high qword).
    const TYPE: u64 = 1 << 62;
    /// Fault reason, bits 103:96 (bits 39:32 of the high qword).
    const REASON_SHIFT: u32 = 32;
    const REASON_MASK: u64 = 0xFF;
    /// Source identifier, bits 79:64 (bits 15:0 of the high qword).
    const SID_MASK: u64 = 0xFFFF;

    /// Whether this register records a fault (F).
    pub const fn is_fault(self) -> bool {
        (self.hi & Self::FAULT) != 0
    }

    /// The source identifier (requester BDF) of the faulting request.
    pub const fn source_id(self) -> u16 {
        (self.hi & Self::SID_MASK) as u16
    }

    /// The fault reason code (FR).
    pub const fn fault_reason(self) -> u8 {
        ((self.hi >> Self::REASON_SHIFT) & Self::REASON_MASK) as u8
    }

    /// The faulted address (FI): bits 63:12 of the record, page-aligned.
    pub const fn fault_address(self) -> u64 {
        self.lo & ADDRESS_MASK
    }

    /// Whether the T bit (126) marks a read, so clear is a device write; AT is not decoded.
    pub const fn is_read_request(self) -> bool {
        (self.hi & Self::TYPE) != 0
    }

    /// The value whose write-1-to-clear bit 127 clears F; the other fields are read-only.
    pub const fn clear() -> Self {
        Self {
            lo: 0,
            hi: Self::FAULT,
        }
    }
}

#[cfg(test)]
mod tests;
