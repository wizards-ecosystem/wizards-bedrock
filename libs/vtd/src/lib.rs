//! Intel VT-d DMA and interrupt-remapping register, table and invalidation encoding.
//!
//! Pure arithmetic over the register window and the kernel-owned root, context
//! and second-level tables. No hardware behind it: `wost`'s VT-d unit maps the
//! register window as MMIO and writes the frames this crate encodes, on the
//! `x86-paging`/`frame-space` precedent (D-30, D-43) that puts checkable bit
//! layout where `cargo test` can reach it.
//!
//! Transcribed from the Intel Virtualization Technology for Directed I/O
//! Architecture Specification, rev 3.x/4.x: the Capability, Extended
//! Capability, Global Command/Status, Root Table Address, Context Command,
//! IOTLB, Fault Status and Fault Recording registers, and the root, context and
//! second-level table entries. Only the register-based invalidation interface
//! is encoded; the unit disables queued invalidation before it uses the
//! Context Command and IOTLB registers, because the specification reserves them
//! while queued invalidation is enabled (D-358). Every field's shift and mask is
//! a named constant, and the tests restate each bit position from the
//! specification rather than from these constants, so a transcription error
//! fails a test instead of agreeing with itself.
//!
//! ```
//! use vtd::{AddressWidth, Capability, ContextEntry, RootEntry, enable_translation, gcmd, second_level};
//!
//! // A capability word advertising 4-level paging (SAGAW bit 2) and a 48-bit
//! // maximum guest address width (MGAW field 47).
//! let cap = Capability::from_bits((1 << (8 + 2)) | (47 << 16));
//! let width = AddressWidth::choose(cap).expect("4-level fits within MGAW");
//! assert_eq!(width.levels(), 4);
//! assert!(width.covers(0xFFFF_FFFF_FFFF) && !width.covers(1 << 48));
//!
//! // Root and context entries for domain 7, and a writable second-level leaf.
//! let root = RootEntry::try_present(0x1000).expect("page aligned");
//! assert_eq!(root.context_table_address(), 0x1000);
//! let context = ContextEntry::try_present(0x2000, width, 7).expect("page aligned");
//! assert_eq!(context.domain_id(), 7);
//! assert_eq!(second_level::pte(0x3000, true, true), 0x3000 | 0b11);
//!
//! // Commands OR their bit into a preserved command word.
//! assert_eq!(enable_translation(0) & gcmd::TE, gcmd::TE);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

/// Physical-address field of a table entry: bits 63:12, page-aligned.
///
/// Every root, context and second-level pointer stores a 4-KiB-aligned
/// physical address in these bits; the low twelve carry flags. Masking with
/// this keeps the address and clears the flag bits, so a caller cannot smuggle
/// a flag through a misaligned pointer.
const ADDRESS_MASK: u64 = 0xFFFF_FFFF_FFFF_F000;

// ---------------------------------------------------------------------------
// Register offsets, in bytes from the unit's register base.
// ---------------------------------------------------------------------------

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Version(u32);

impl Version {
    /// Build a version view over the raw Version register.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The raw Version register.
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

    /// Whether this architecture version executes register-based invalidation.
    ///
    /// Major versions six and later require queued invalidation and treat the
    /// register commands as invalid. Zero is not an implemented architecture
    /// version, so rejecting it keeps malformed register windows fail-closed.
    pub const fn register_invalidation_supported(self) -> bool {
        matches!(self.major(), 1..=5)
    }
}

/// Fault Event Control register bits.
///
/// VT-d fault events are compatibility-format messages that do not traverse
/// interrupt remapping. The owner therefore keeps the event source masked for
/// its complete lifetime and polls the fault records instead.
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

/// The root-table pointer the unit walks, read back from the register.
///
/// Bits 63:12 hold the 4-KiB-aligned table address; bits 11:10 select the
/// translation-table mode, which this crate leaves at legacy mode (zero).
pub const fn root_table_address(rtaddr: u64) -> u64 {
    rtaddr & ADDRESS_MASK
}

// ---------------------------------------------------------------------------
// Capability register (CAP).
// ---------------------------------------------------------------------------

/// The Capability register: what the remapping unit can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// Five SAGAW bits.
    const SAGAW_MASK: u64 = 0x1F;
    /// Maximum guest address width, bits 21:16.
    const MGAW_SHIFT: u32 = 16;
    /// Six MGAW bits.
    const MGAW_MASK: u64 = 0x3F;
    /// Fault-recording register offset, bits 33:24.
    const FRO_SHIFT: u32 = 24;
    /// Ten FRO bits.
    const FRO_MASK: u64 = 0x3FF;
    /// Number of fault-recording registers, bits 47:40.
    const NFR_SHIFT: u32 = 40;
    /// Eight NFR bits.
    const NFR_MASK: u64 = 0xFF;
    /// Write draining support, bit 54.
    const DWD: u64 = 1 << 54;
    /// Read draining support, bit 55.
    const DRD: u64 = 1 << 55;
    /// Enhanced Set Interrupt Remap Table Pointer support, bit 62.
    const ESIRTPS: u64 = 1 << 62;

    /// Build a capability view over a raw register value.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The raw register value.
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

    /// Whether the unit uses caching mode (CM): not-present and invalid entries
    /// may be cached, so software must invalidate after every such change.
    pub const fn caching_mode(self) -> bool {
        (self.0 & Self::CM) != 0
    }

    /// Whether the SAGAW bit for a `levels`-level page table is set.
    ///
    /// SAGAW bit `levels - 2` is the adjacent guest address width: bit 0 is
    /// 30-bit/2-level, bit 1 39-bit/3-level, bit 2 48-bit/4-level, bit 3
    /// 57-bit/5-level.
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

// ---------------------------------------------------------------------------
// Extended Capability register (ECAP).
// ---------------------------------------------------------------------------

/// The Extended Capability register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// Ten IRO bits.
    const IRO_MASK: u64 = 0x3FF;

    /// Build an extended-capability view over a raw register value.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The raw register value.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Whether page-table walks are coherent with processor caches (C).
    ///
    /// When clear, the unit may read a table entry from memory before the
    /// processor's cached write reaches it, so every entry write must be
    /// followed by a cache-line flush before the unit can be relied on.
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
    ///
    /// Within that block the IOTLB Invalidate register (IOTLB_REG) sits at
    /// [`iotlb_command::COMMAND_REGISTER`].
    pub const fn iotlb_register_offset_bytes(self) -> u64 {
        ((self.0 >> Self::IRO_SHIFT) & Self::IRO_MASK) * 16
    }
}

// ---------------------------------------------------------------------------
// Global Command / Global Status.
// ---------------------------------------------------------------------------

/// Global Command register bits.
///
/// GCMD is driven from a software-preserved shadow: each write sets one command
/// bit while leaving the other enable bits in their intended state.
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
pub const fn set_root_table_pointer(cmd: u32) -> u32 {
    cmd | gcmd::SRTP
}

/// OR the Translation-Enable bit into a preserved command word.
pub const fn enable_translation(cmd: u32) -> u32 {
    cmd | gcmd::TE
}

/// OR the Set-Interrupt-Remap-Table-Pointer bit into a command word.
pub const fn set_interrupt_table_pointer(cmd: u32) -> u32 {
    cmd | gcmd::SIRTP
}

/// Return a command word with remapping on and compatibility-format delivery off.
pub const fn enable_interrupt_remapping(cmd: u32) -> u32 {
    (cmd | gcmd::IRE) & !gcmd::CFI
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

/// Whether every firmware-owned remapping facility is already inactive.
///
/// An early kernel owner may program a non-identical root only from this
/// state. It refuses active translation instead of attempting a live SRTP,
/// and refuses active queued or interrupt remapping instead of guessing that
/// firmware completed their architectural quiescence protocols.
pub const fn takeover_ready(status: u32) -> bool {
    status & (gsts::TES | gsts::QIES | gsts::IRES | gsts::CFIS) == 0
}

// ---------------------------------------------------------------------------
// Interrupt remapping and queued invalidation.
// ---------------------------------------------------------------------------

/// Address bits carried by IRTA and IQA.
const REMAPPING_ADDRESS_MASK: u64 = 0xFFFF_FFFF_FFFF_F000;

/// Encode a two-entry, 128-bit interrupt-remapping table at `address`.
///
/// Size zero means two entries. EIME is set because this kernel admits only
/// x2APIC interrupt delivery; extended mode also blocks compatibility-format
/// requests independently of CFI.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidationDescriptor {
    /// Lower 64 bits.
    pub lo: u64,
    /// Upper 64 bits.
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

// ---------------------------------------------------------------------------
// Address width, AGAW selection, and paging-structure indexing.
// ---------------------------------------------------------------------------

/// A supported adjusted guest address width and its page-table depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressWidth {
    /// 39-bit addresses, a 3-level second-level page table.
    Level3,
    /// 48-bit addresses, a 4-level second-level page table.
    Level4,
    /// 57-bit addresses, a 5-level second-level page table.
    Level5,
}

impl AddressWidth {
    /// The page-table depth: 3, 4 or 5 levels.
    pub const fn levels(self) -> u8 {
        match self {
            Self::Level3 => 3,
            Self::Level4 => 4,
            Self::Level5 => 5,
        }
    }

    /// Addressable width in bits: 39, 48 or 57.
    pub const fn address_bits(self) -> u8 {
        match self {
            Self::Level3 => 39,
            Self::Level4 => 48,
            Self::Level5 => 57,
        }
    }

    /// Whether `address` can be named by a walk of this width.
    ///
    /// An address above the width would alias into the top table's index bits
    /// if it were mapped, so a caller refuses it instead.
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
    pub const fn covers_on(self, cap: Capability, address: u64) -> bool {
        let bits = cap.max_guest_address_width();
        self.covers(address) && (bits == 64 || address >> bits == 0)
    }

    /// Choose the widest modelled table depth supported by SAGAW.
    ///
    /// MGAW independently limits usable addresses through [`Self::covers_on`];
    /// it does not prohibit a deeper supported table. The 2-level width is not
    /// modelled here.
    pub const fn choose(cap: Capability) -> Option<Self> {
        if cap.sagaw_supports(5) {
            return Some(Self::Level5);
        }
        if cap.sagaw_supports(4) {
            return Some(Self::Level4);
        }
        if cap.sagaw_supports(3) {
            return Some(Self::Level3);
        }
        None
    }

    /// The 9-bit index `iova` selects at paging-structure `level`.
    ///
    /// Level 0 is the leaf page table; each level up is nine bits higher. The
    /// split points (12, 21, 30, 39, 48) are the standard x86-style ones and do
    /// not depend on the width, which only bounds how many levels a walk uses,
    /// so a caller iterates `0..self.levels()`.
    pub const fn index(self, iova: u64, level: u8) -> u16 {
        let shift = 12 + 9 * (level as u32);
        ((iova >> shift) & 0x1FF) as u16
    }
}

// ---------------------------------------------------------------------------
// Root and context table entries.
// ---------------------------------------------------------------------------

/// A root-table entry: the low and high 64-bit words a root entry occupies.
///
/// Stored little-endian: [`lo`](Self::lo) is written at the entry's base and
/// [`hi`](Self::hi) eight bytes above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RootEntry {
    /// Low qword: present bit and the context-table pointer.
    pub lo: u64,
    /// High qword: reserved, always zero.
    pub hi: u64,
}

impl RootEntry {
    /// Present, bit 0.
    const PRESENT: u64 = 1 << 0;

    /// A present root entry naming a context table.
    ///
    /// `context_table_paddr` must be 4-KiB aligned; its low twelve bits are
    /// masked off, so a misaligned pointer silently loses them. Use
    /// [`try_present`](Self::try_present) to reject a misaligned pointer.
    #[must_use]
    pub const fn present(context_table_paddr: u64) -> Self {
        Self {
            lo: (context_table_paddr & ADDRESS_MASK) | Self::PRESENT,
            hi: 0,
        }
    }

    /// A present root entry, or `None` if `context_table_paddr` is not 4-KiB
    /// aligned.
    pub const fn try_present(context_table_paddr: u64) -> Option<Self> {
        if context_table_paddr & 0xFFF != 0 {
            return None;
        }
        Some(Self::present(context_table_paddr))
    }

    /// Whether the entry is present.
    pub const fn is_present(self) -> bool {
        (self.lo & Self::PRESENT) != 0
    }

    /// The context-table physical address the entry names.
    pub const fn context_table_address(self) -> u64 {
        self.lo & ADDRESS_MASK
    }
}

/// A context-table entry: the low and high 64-bit words, little-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

    /// A present context entry for legacy second-level-only translation.
    ///
    /// The translation-type field (bits 3:2 of the low qword) is left zero,
    /// which selects second-level-only translation. `second_level_root_paddr`
    /// must be 4-KiB aligned; its low twelve bits are masked off. Use
    /// [`try_present`](Self::try_present) to reject a misaligned pointer.
    #[must_use]
    pub const fn present(second_level_root_paddr: u64, aw: AddressWidth, domain_id: u16) -> Self {
        Self {
            lo: (second_level_root_paddr & ADDRESS_MASK) | Self::PRESENT,
            hi: aw.context_encoding() | ((domain_id as u64) << Self::DID_SHIFT),
        }
    }

    /// A present context entry, or `None` if `second_level_root_paddr` is not
    /// 4-KiB aligned.
    pub const fn try_present(
        second_level_root_paddr: u64,
        aw: AddressWidth,
        domain_id: u16,
    ) -> Option<Self> {
        if second_level_root_paddr & 0xFFF != 0 {
            return None;
        }
        Some(Self::present(second_level_root_paddr, aw, domain_id))
    }

    /// Whether the entry is present.
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

// ---------------------------------------------------------------------------
// Second-level (page table) entries.
// ---------------------------------------------------------------------------

/// Second-level page-table entries: leaves and table pointers.
///
/// A second-level structure encodes read/write permission in bits 1:0 and a
/// 4-KiB-aligned physical address in bits 63:12. A leaf and a non-leaf pointer
/// share that layout, so an intermediate
/// [`table_pointer`](second_level::table_pointer) is `R | W | addr`.
pub mod second_level {
    use super::ADDRESS_MASK;

    /// Read permission, bit 0.
    const READ: u64 = 1 << 0;
    /// Write permission, bit 1.
    const WRITE: u64 = 1 << 1;

    /// An all-zero, not-present entry.
    pub const NOT_PRESENT: u64 = 0;

    /// A leaf entry mapping `paddr` with the given permissions.
    ///
    /// `paddr`'s low twelve bits are masked off.
    pub const fn pte(paddr: u64, read: bool, write: bool) -> u64 {
        let mut bits = paddr & ADDRESS_MASK;
        if read {
            bits |= READ;
        }
        if write {
            bits |= WRITE;
        }
        bits
    }

    /// A non-leaf entry pointing at the next-level table at `paddr`.
    ///
    /// Read and write are both set, so the walk's permission is decided at the
    /// leaves. `paddr`'s low twelve bits are masked off.
    pub const fn table_pointer(paddr: u64) -> u64 {
        (paddr & ADDRESS_MASK) | READ | WRITE
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

// ---------------------------------------------------------------------------
// Register-based invalidation: Context Command (CCMD) and IOTLB.
// ---------------------------------------------------------------------------

/// Context-cache invalidation through the Context Command register (CCMD).
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

    /// A global context-cache invalidation command word.
    pub const fn global() -> u64 {
        ICC | GLOBAL
    }

    /// Whether a read-back value shows the invalidation complete (ICC clear).
    pub const fn completed(value: u64) -> bool {
        (value & ICC) == 0
    }

    /// Whether a global request completed and hardware reports actually
    /// performing a global context-cache invalidation.
    pub const fn global_completed(value: u64) -> bool {
        completed(value) && ((value >> CAIG_SHIFT) & CAIG_MASK) == ACTUAL_GLOBAL
    }
}

/// IOTLB invalidation through the IOTLB Invalidate register (IOTLB_REG).
///
/// The register sits at [`COMMAND_REGISTER`](iotlb_command::COMMAND_REGISTER)
/// within the IOTLB register block that
/// [`ExtendedCapability::iotlb_register_offset_bytes`] locates.
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

    /// A global IOTLB invalidation command word.
    pub const fn global() -> u64 {
        IVT | GLOBAL
    }

    /// A global invalidation that completes only after older DMA reads and
    /// writes have drained.
    ///
    /// The caller must first require the matching DRD and DWD capability bits.
    pub const fn global_with_drain() -> u64 {
        global() | DRAIN_READS | DRAIN_WRITES
    }

    /// Whether a read-back value shows the invalidation complete (IVT clear).
    pub const fn completed(value: u64) -> bool {
        (value & IVT) == 0
    }

    /// Whether a global request completed and hardware reports actually
    /// performing a global IOTLB invalidation.
    pub const fn global_completed(value: u64) -> bool {
        completed(value) && ((value >> IAIG_SHIFT) & IAIG_MASK) == ACTUAL_GLOBAL
    }
}

// ---------------------------------------------------------------------------
// Fault decoding: Fault Status (FSTS) and Fault Recording registers.
// ---------------------------------------------------------------------------

/// The Fault Status register (FSTS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FaultStatus(u32);

impl FaultStatus {
    /// Primary fault overflow, bit 0.
    const PFO: u32 = 1 << 0;
    /// Primary pending fault, bit 1.
    const PPF: u32 = 1 << 1;
    /// Fault record index, bits 15:8.
    const FRI_SHIFT: u32 = 8;
    /// Eight FRI bits.
    const FRI_MASK: u32 = 0xFF;

    /// Build a fault-status view over a raw register value.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// Whether a primary fault is pending (PPF).
    pub const fn primary_pending(self) -> bool {
        (self.0 & Self::PPF) != 0
    }

    /// Whether the fault log overflowed (PFO).
    ///
    /// While set, the unit records no new fault, so a reader must clear it with
    /// [`acknowledge_overflow`](Self::acknowledge_overflow) to see later ones.
    pub const fn overflow(self) -> bool {
        (self.0 & Self::PFO) != 0
    }

    /// Index of the fault-recording register holding the first pending fault.
    pub const fn fault_record_index(self) -> u8 {
        ((self.0 >> Self::FRI_SHIFT) & Self::FRI_MASK) as u8
    }

    /// The value to write back to clear an overflow: PFO is write-1-to-clear,
    /// and nothing else in the register is written.
    pub const fn acknowledge_overflow() -> u32 {
        Self::PFO
    }
}

/// A 16-byte fault-recording register: the low and high 64-bit words.
///
/// Bit numbering is over the whole 128 bits: `lo` holds bits 63:0 and `hi` bits
/// 127:64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// Eight fault-reason bits.
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

    /// Whether the faulting request was a read (the T bit).
    ///
    /// The T field (bit 126) distinguishes the request type; this crate reads a
    /// set bit as a read request, so a clear bit is a device write into host
    /// memory. Some specification revisions also define an address-type field
    /// in the neighbouring bits, which is not decoded here.
    pub const fn is_read_request(self) -> bool {
        (self.hi & Self::TYPE) != 0
    }

    /// The value to write back to clear a recorded fault.
    ///
    /// The fault bit is write-1-to-clear, so writing this (the high qword with
    /// bit 127 set) clears F; the other fields are read-only and ignored.
    pub const fn clear() -> Self {
        Self {
            lo: 0,
            hi: Self::FAULT,
        }
    }
}

#[cfg(test)]
mod tests;
