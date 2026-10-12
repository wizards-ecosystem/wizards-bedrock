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
    /// Fault reason, bits 111:104 (bits 47:40 of the high qword).
    const REASON_SHIFT: u32 = 40;
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
mod tests {
    use super::*;

    #[test]
    fn version_bounds_the_register_invalidation_interface() {
        assert_eq!(VERSION, 0);
        assert_eq!(Version::from_bits(0x51).bits(), 0x51);
        assert_eq!(Version::from_bits(0x51).major(), 5);
        assert_eq!(Version::from_bits(0x51).minor(), 1);
        for major in 1..=5 {
            assert!(Version::from_bits(major << 4).register_invalidation_supported());
        }
        assert!(!Version::from_bits(0).register_invalidation_supported());
        assert!(!Version::from_bits(6 << 4).register_invalidation_supported());
        assert!(!Version::from_bits(15 << 4).register_invalidation_supported());
    }

    #[test]
    fn register_offsets_are_the_spec_byte_offsets() {
        assert_eq!(CAP, 0x08);
        assert_eq!(ECAP, 0x10);
        assert_eq!(GCMD, 0x18);
        assert_eq!(GSTS, 0x1C);
        assert_eq!(RTADDR, 0x20);
        assert_eq!(CCMD, 0x28);
        assert_eq!(FSTS, 0x34);
        assert_eq!(FECTL, 0x38);
    }

    #[test]
    fn fault_event_control_masks_the_non_remapped_message_source() {
        assert_eq!(fault_event_control::PENDING, 1 << 30);
        assert_eq!(fault_event_control::MASK, 1 << 31);
        assert!(!fault_event_control::masked(0));
        assert!(!fault_event_control::masked(1 << 30));
        assert!(fault_event_control::masked(1 << 31));
        assert!(fault_event_control::masked((1 << 31) | (1 << 30)));
    }

    /// The address field is bits 63:12 (52 bits), independent of the constant.
    #[test]
    fn the_entry_address_field_is_bits_63_through_12() {
        assert_eq!(ADDRESS_MASK, !0xFFFu64);
        assert_eq!(ADDRESS_MASK.trailing_zeros(), 12);
        assert_eq!(ADDRESS_MASK.count_ones(), 52);
    }

    #[test]
    fn the_root_table_address_drops_the_mode_and_reserved_bits() {
        assert_eq!(
            root_table_address(0x1234_5000 | 0b1100_0000_0000),
            0x1234_5000
        );
        assert_eq!(root_table_address(0x1234_5FFF), 0x1234_5000);
        assert_eq!(root_table_address(0), 0);
    }

    /// Capability field positions, restated from the spec rather than from the
    /// constants above.
    #[test]
    fn capability_field_positions_match_the_spec() {
        assert_eq!(Capability::ND_MASK, 0x7, "ND bits 2:0");
        assert_eq!(Capability::RWBF, 1 << 4);
        assert_eq!(Capability::CM, 1 << 7);
        assert_eq!(Capability::SAGAW_SHIFT, 8);
        assert_eq!(Capability::SAGAW_MASK, 0x1F, "SAGAW bits 12:8");
        assert_eq!(Capability::MGAW_SHIFT, 16);
        assert_eq!(Capability::MGAW_MASK, 0x3F, "MGAW bits 21:16");
        assert_eq!(Capability::FRO_SHIFT, 24);
        assert_eq!(Capability::FRO_MASK, 0x3FF, "FRO bits 33:24");
        assert_eq!(Capability::NFR_SHIFT, 40);
        assert_eq!(Capability::NFR_MASK, 0xFF, "NFR bits 47:40");
        assert_eq!(Capability::DWD, 1 << 54);
        assert_eq!(Capability::DRD, 1 << 55);
    }

    #[test]
    fn capability_decodes_each_field_from_its_own_bits() {
        // Pairwise-distinct field values placed with independent literal shifts.
        let cap = Capability::from_bits(
            5 | (1 << 4)
                | (1 << 7)
                | (4 << 8)
                | (47 << 16)
                | (0x123u64 << 24)
                | (0x12u64 << 40)
                | (1 << 54)
                | (1 << 55),
        );
        assert_eq!(cap.number_of_domains(), 1 << (4 + 2 * 5)); // ND = 5
        assert!(cap.requires_write_buffer_flush());
        assert!(cap.caching_mode());
        assert!(cap.sagaw_supports(4));
        assert!(!cap.sagaw_supports(3));
        assert!(!cap.sagaw_supports(5));
        assert!(!cap.sagaw_supports(2));
        assert_eq!(cap.max_guest_address_width(), 48); // MGAW field 47
        assert_eq!(cap.fault_recording_offset_bytes(), 0x123 * 16);
        assert_eq!(cap.number_of_fault_recording_registers(), 0x12 + 1);
        assert!(cap.write_draining());
        assert!(cap.read_draining());
        assert!(cap.dma_draining());
        assert_eq!(cap.bits() & 0x7, 5);
    }

    #[test]
    fn number_of_domains_follows_the_power_of_two_formula() {
        assert_eq!(Capability::from_bits(0).number_of_domains(), 16); // 2^4
        assert_eq!(Capability::from_bits(1).number_of_domains(), 64); // 2^6
        assert_eq!(Capability::from_bits(3).number_of_domains(), 1 << 10);
        assert_eq!(Capability::from_bits(7).number_of_domains(), 1 << 18);
    }

    #[test]
    fn write_buffer_flush_and_caching_mode_are_read_independently() {
        assert!(!Capability::from_bits(1 << 7).requires_write_buffer_flush());
        assert!(Capability::from_bits(1 << 7).caching_mode());
        assert!(Capability::from_bits(1 << 4).requires_write_buffer_flush());
        assert!(!Capability::from_bits(1 << 4).caching_mode());
    }

    #[test]
    fn dma_draining_requires_both_independent_capability_bits() {
        assert!(!Capability::from_bits(0).dma_draining());
        assert!(!Capability::from_bits(1 << 54).dma_draining());
        assert!(!Capability::from_bits(1 << 55).dma_draining());
        assert!(Capability::from_bits((1 << 54) | (1 << 55)).dma_draining());
        assert!(!Capability::from_bits(1 << 61).enhanced_set_interrupt_table_pointer());
        assert!(Capability::from_bits(1 << 62).enhanced_set_interrupt_table_pointer());
    }

    #[test]
    fn sagaw_maps_each_level_to_its_own_bit() {
        // Bit (levels - 2), placed with an independent literal.
        for levels in 2u8..=5 {
            let cap = Capability::from_bits(1 << (8 + (levels - 2) as u64));
            assert!(cap.sagaw_supports(levels), "level {levels}");
            for other in 2u8..=5 {
                if other != levels {
                    assert!(!cap.sagaw_supports(other), "level {levels} leaked {other}");
                }
            }
        }
        assert!(!Capability::from_bits(0).sagaw_supports(4));
        assert!(!Capability::from_bits(u64::MAX).sagaw_supports(1));
        assert!(!Capability::from_bits(u64::MAX).sagaw_supports(7));
    }

    #[test]
    fn extended_capability_field_positions_match_the_spec() {
        assert_eq!(ExtendedCapability::C, 1 << 0);
        assert_eq!(ExtendedCapability::QI, 1 << 1);
        assert_eq!(ExtendedCapability::IR, 1 << 3);
        assert_eq!(ExtendedCapability::EIM, 1 << 4);
        assert_eq!(ExtendedCapability::IRO_SHIFT, 8);
        assert_eq!(ExtendedCapability::IRO_MASK, 0x3FF, "IRO bits 17:8");
    }

    #[test]
    fn extended_capability_decodes_each_field() {
        let ecap =
            ExtendedCapability::from_bits(1 | (1 << 1) | (1 << 3) | (1 << 4) | (0x2Au64 << 8));
        assert!(ecap.page_walk_coherent());
        assert!(ecap.queued_invalidation());
        assert!(ecap.interrupt_remapping());
        assert!(ecap.extended_interrupt_mode());
        assert_eq!(ecap.iotlb_register_offset_bytes(), 0x2A * 16);

        let only_ir = ExtendedCapability::from_bits(1 << 3);
        assert!(only_ir.interrupt_remapping());
        assert!(!only_ir.extended_interrupt_mode());
        let only_eim = ExtendedCapability::from_bits(1 << 4);
        assert!(!only_eim.interrupt_remapping());
        assert!(only_eim.extended_interrupt_mode());

        // Only IRO set: the three single-bit fields read false.
        let only_iro = ExtendedCapability::from_bits(0x2Au64 << 8);
        assert!(!only_iro.page_walk_coherent());
        assert!(!only_iro.queued_invalidation());
        assert!(!only_iro.interrupt_remapping());
        assert!(!only_iro.extended_interrupt_mode());
        assert_eq!(only_iro.iotlb_register_offset_bytes(), 0x2A * 16);
        assert_eq!(only_iro.bits(), 0x2Au64 << 8);
    }

    #[test]
    fn global_command_and_status_bits_match_the_spec() {
        assert_eq!(gcmd::TE, 1 << 31);
        assert_eq!(gcmd::SRTP, 1 << 30);
        assert_eq!(gcmd::WBF, 1 << 27);
        assert_eq!(gcmd::QIE, 1 << 26);
        assert_eq!(gcmd::IRE, 1 << 25);
        assert_eq!(gcmd::SIRTP, 1 << 24);
        assert_eq!(gcmd::CFI, 1 << 23);
        assert_eq!(gsts::TES, 1 << 31);
        assert_eq!(gsts::RTPS, 1 << 30);
        assert_eq!(gsts::WBFS, 1 << 27);
        assert_eq!(gsts::QIES, 1 << 26);
        assert_eq!(gsts::IRES, 1 << 25);
        assert_eq!(gsts::IRTPS, 1 << 24);
        assert_eq!(gsts::CFIS, 1 << 23);
    }

    #[test]
    fn command_helpers_or_one_bit_into_a_preserved_word() {
        let preserved = gcmd::IRE | gcmd::TE;
        assert_eq!(set_root_table_pointer(preserved), preserved | gcmd::SRTP);
        assert_eq!(enable_translation(0), gcmd::TE);
        assert_eq!(set_interrupt_table_pointer(0), 1 << 24);
        assert_eq!(enable_interrupt_remapping(gcmd::CFI), 1 << 25);
        // The preserved bits survive.
        assert_eq!(enable_translation(gcmd::IRE) & gcmd::IRE, gcmd::IRE);
    }

    #[test]
    fn status_predicates_read_their_own_bit() {
        assert!(root_table_pointer_set(gsts::RTPS));
        assert!(!root_table_pointer_set(gsts::TES));
        assert!(translation_enabled(gsts::TES));
        assert!(!translation_enabled(gsts::RTPS));
        assert!(interrupt_table_pointer_set(gsts::IRTPS));
        assert!(!interrupt_table_pointer_set(gsts::IRES));
        assert!(interrupt_remapping_enabled(gsts::IRES));
        assert!(!interrupt_remapping_enabled(gsts::IRTPS));
        assert!(compatibility_interrupts_enabled(gsts::CFIS));
        assert!(!compatibility_interrupts_enabled(gsts::IRES));
    }

    #[test]
    fn takeover_requires_every_firmware_facility_to_be_inactive() {
        assert!(takeover_ready(0));
        assert!(takeover_ready(gsts::RTPS | gsts::IRTPS));
        assert!(!takeover_ready(gsts::TES));
        assert!(!takeover_ready(gsts::QIES));
        assert!(!takeover_ready(gsts::IRES));
        assert!(!takeover_ready(gsts::CFIS));
        assert!(!takeover_ready(u32::MAX));
    }

    #[test]
    fn interrupt_table_and_queue_addresses_are_exact_and_aligned() {
        assert_eq!(IRTA, 0xB8);
        assert_eq!(IQA, 0x90);
        assert_eq!(IQH, 0x80);
        assert_eq!(IQT, 0x88);
        assert_eq!(interrupt_table_address(0x1234_5000), Some(0x1234_5800));
        assert_eq!(interrupt_table_address(0x1234_5001), None);
        assert_eq!(interrupt_table_base(0x1234_580f), 0x1234_5000);
        assert!(extended_interrupt_mode_enabled(0x800));
        assert!(interrupt_table_has_two_entries(0x1234_5800));
        assert!(!interrupt_table_has_two_entries(0x1234_5801));
        assert_eq!(invalidation_queue_address(0x2234_5000), Some(0x2234_5000));
        assert_eq!(invalidation_queue_address(0x2234_5008), None);
    }

    #[test]
    fn queued_interrupt_cache_invalidation_descriptors_match_the_spec() {
        assert_eq!(
            InvalidationDescriptor::global_interrupt_entry_cache(),
            InvalidationDescriptor { lo: 4, hi: 0 }
        );
        assert_ne!(
            InvalidationDescriptor::global_interrupt_entry_cache().lo,
            4 | (1 << 4),
            "IEC granularity bit 4 selects one index rather than the global cache"
        );
        assert_eq!(
            InvalidationDescriptor::wait_status(0x4000, 0xa5a5_5a5a),
            Some(InvalidationDescriptor {
                lo: 5 | (1 << 5) | (0xa5a5_5a5au64 << 32),
                hi: 0x4000,
            })
        );
        assert_eq!(InvalidationDescriptor::wait_status(0x4002, 1), None);
    }

    #[test]
    fn address_width_knows_its_depth_and_encoding() {
        assert_eq!(AddressWidth::Level3.levels(), 3);
        assert_eq!(AddressWidth::Level4.levels(), 4);
        assert_eq!(AddressWidth::Level5.levels(), 5);
        assert_eq!(AddressWidth::Level3.address_bits(), 39);
        assert_eq!(AddressWidth::Level4.address_bits(), 48);
        assert_eq!(AddressWidth::Level5.address_bits(), 57);
        assert_eq!(AddressWidth::Level3.context_encoding(), 1);
        assert_eq!(AddressWidth::Level4.context_encoding(), 2);
        assert_eq!(AddressWidth::Level5.context_encoding(), 3);
    }

    /// Coverage ends exactly at each width's power of two, on both sides.
    #[test]
    fn a_width_covers_every_address_below_its_power_of_two_and_none_above() {
        for (width, bits) in [
            (AddressWidth::Level3, 39u32),
            (AddressWidth::Level4, 48),
            (AddressWidth::Level5, 57),
        ] {
            assert!(width.covers(0));
            assert!(width.covers((1u64 << bits) - 1), "{bits}-bit top page");
            assert!(!width.covers(1u64 << bits), "{bits}-bit overflow");
            assert!(!width.covers(u64::MAX));
        }
        // A 48-bit walk covers an address a 39-bit walk refuses.
        assert!(AddressWidth::Level4.covers(1 << 39));
        assert!(!AddressWidth::Level3.covers(1 << 39));
    }

    #[test]
    fn choose_prefers_the_widest_supported_table() {
        // A large MGAW so only SAGAW decides.
        let big_mgaw = 62u64 << 16; // MGAW field 62 -> 63-bit
        let all = Capability::from_bits((1 << 9) | (1 << 10) | (1 << 11) | big_mgaw);
        assert_eq!(AddressWidth::choose(all), Some(AddressWidth::Level5));

        let three_four = Capability::from_bits((1 << 9) | (1 << 10) | big_mgaw);
        assert_eq!(AddressWidth::choose(three_four), Some(AddressWidth::Level4));

        let three = Capability::from_bits((1 << 9) | big_mgaw);
        assert_eq!(AddressWidth::choose(three), Some(AddressWidth::Level3));

        // The 2-level width is supported but never chosen.
        let two = Capability::from_bits((1 << 8) | big_mgaw);
        assert_eq!(AddressWidth::choose(two), None);
        assert_eq!(AddressWidth::choose(Capability::from_bits(big_mgaw)), None);
    }

    #[test]
    fn supported_table_depth_is_independent_of_guest_address_limit() {
        for sagaw in 0u64..32 {
            let expected = [
                (3, AddressWidth::Level5),
                (2, AddressWidth::Level4),
                (1, AddressWidth::Level3),
            ]
            .into_iter()
            .find_map(|(bit, width)| (sagaw & (1 << bit) != 0).then_some(width));
            for mgaw_field in 0u64..64 {
                let cap = Capability::from_bits((sagaw << 8) | (mgaw_field << 16));
                assert_eq!(
                    AddressWidth::choose(cap),
                    expected,
                    "{sagaw:#x}/{mgaw_field}"
                );
            }
        }
        let four_only = Capability::from_bits((1 << 10) | (38 << 16));
        assert_eq!(AddressWidth::choose(four_only), Some(AddressWidth::Level4));
    }

    #[test]
    fn effective_coverage_obeys_both_table_and_guest_limits() {
        for (width, table_bits) in [
            (AddressWidth::Level3, 39u32),
            (AddressWidth::Level4, 48),
            (AddressWidth::Level5, 57),
        ] {
            for guest_bits in 1u32..=64 {
                let cap = Capability::from_bits(u64::from(guest_bits - 1) << 16);
                let boundary = 1u64 << table_bits.min(guest_bits);
                assert!(width.covers_on(cap, 0));
                assert!(width.covers_on(cap, boundary - 1));
                assert!(!width.covers_on(cap, boundary));
                assert!(!width.covers_on(cap, u64::MAX));
            }
        }
    }

    #[test]
    fn index_splits_an_iova_nine_bits_at_a_time() {
        // Distinct multi-bit indices placed with independent literal shifts.
        let iova =
            0xABC | (0x1F << 12) | (0x2E << 21) | (0x3D << 30) | (0x4Cu64 << 39) | (0x5Bu64 << 48);
        let w = AddressWidth::Level5;
        assert_eq!(w.index(iova, 0), 0x1F);
        assert_eq!(w.index(iova, 1), 0x2E);
        assert_eq!(w.index(iova, 2), 0x3D);
        assert_eq!(w.index(iova, 3), 0x4C);
        assert_eq!(w.index(iova, 4), 0x5B);
        // Every index is a 9-bit value.
        assert_eq!(AddressWidth::Level4.index(u64::MAX, 0), 0x1FF);
    }

    #[test]
    fn a_root_entry_carries_present_and_a_masked_pointer() {
        let entry = RootEntry::present(0x1234_5000);
        assert_eq!(entry.lo, 0x1234_5000 | 1);
        assert_eq!(entry.hi, 0);
        assert!(entry.is_present());
        assert_eq!(entry.context_table_address(), 0x1234_5000);

        // A misaligned pointer loses its low bits but sets only bit 0 below 12.
        let masked = RootEntry::present(0x1234_5FFF);
        assert_eq!(masked.context_table_address(), 0x1234_5000);
        assert_eq!(
            masked.lo & 0xFFE,
            0,
            "no reserved low bit but present is set"
        );
        assert!(masked.is_present());
        assert!(!RootEntry::default().is_present());
    }

    #[test]
    fn root_entry_alignment_is_checked_only_by_try_present() {
        assert!(RootEntry::try_present(0x2000).is_some());
        assert_eq!(RootEntry::try_present(0x2001), None);
        assert_eq!(
            RootEntry::try_present(0x2000),
            Some(RootEntry::present(0x2000))
        );
    }

    #[test]
    fn a_context_entry_encodes_pointer_width_and_domain() {
        let entry = ContextEntry::present(0x2000, AddressWidth::Level4, 0xABC);
        assert_eq!(entry.lo, 0x2000 | 1);
        assert_eq!(entry.hi, 2 | (0xABC << 8));
        assert!(entry.is_present());
        assert_eq!(entry.translation_type(), 0, "legacy second-level only");
        assert_eq!(entry.second_level_address(), 0x2000);
        assert_eq!(entry.address_width_encoding(), 2);
        assert_eq!(entry.domain_id(), 0xABC);
    }

    #[test]
    fn context_entry_masks_its_pointer_and_covers_each_width() {
        let masked = ContextEntry::present(0x2FFF, AddressWidth::Level3, 5);
        assert_eq!(masked.second_level_address(), 0x2000);
        assert_eq!(masked.address_width_encoding(), 1);

        for (width, encoding) in [
            (AddressWidth::Level3, 1),
            (AddressWidth::Level4, 2),
            (AddressWidth::Level5, 3),
        ] {
            let entry = ContextEntry::present(0x3000, width, 0xFFFF);
            assert_eq!(entry.address_width_encoding(), encoding);
            assert_eq!(entry.domain_id(), 0xFFFF);
        }

        assert!(!ContextEntry::default().is_present());
        assert_eq!(
            ContextEntry::try_present(0x3001, AddressWidth::Level4, 0),
            None
        );
        assert_eq!(
            ContextEntry::try_present(0x3000, AddressWidth::Level4, 9),
            Some(ContextEntry::present(0x3000, AddressWidth::Level4, 9))
        );
    }

    #[test]
    fn second_level_entries_encode_permission_and_address() {
        assert_eq!(second_level::pte(0x3000, true, true), 0x3000 | 0b11);
        assert_eq!(second_level::pte(0x3000, true, false), 0x3000 | 0b01);
        assert_eq!(second_level::pte(0x3000, false, true), 0x3000 | 0b10);
        assert_eq!(second_level::pte(0x3000, false, false), 0x3000);
        // Read is bit 0, write is bit 1.
        assert_eq!(second_level::pte(0, true, false), 1);
        assert_eq!(second_level::pte(0, false, true), 2);
        // The address is masked to 4 KiB.
        assert_eq!(second_level::pte(0x3FFF, true, true), 0x3000 | 0b11);
        // A table pointer is read | write | address.
        assert_eq!(second_level::table_pointer(0x4000), 0x4000 | 0b11);
        assert_eq!(second_level::table_pointer(0x4FFF), 0x4000 | 0b11);
        assert_eq!(second_level::NOT_PRESENT, 0);
    }

    /// Reading an entry back inverts the encoding: presence is either
    /// permission bit, and the address is the page-aligned remainder.
    #[test]
    fn second_level_entries_read_back_their_presence_and_address() {
        assert!(!second_level::is_present(second_level::NOT_PRESENT));
        assert!(
            !second_level::is_present(0x3000),
            "an address with no permission is absent"
        );
        assert!(second_level::is_present(second_level::pte(
            0x3000, true, false
        )));
        assert!(second_level::is_present(second_level::pte(
            0x3000, false, true
        )));
        assert!(second_level::is_present(second_level::table_pointer(
            0x4000
        )));
        assert_eq!(
            second_level::address(second_level::pte(0x3000, true, true)),
            0x3000
        );
        assert_eq!(
            second_level::address(second_level::table_pointer(0x4000)),
            0x4000
        );
        assert_eq!(second_level::address(0xFFFF_FFFF_FFFF_FFFF), ADDRESS_MASK);
    }

    #[test]
    fn context_command_words_and_readback() {
        assert_eq!(context_command::global(), (1 << 63) | (1 << 61));
        // ICC clear means complete; a fresh command has ICC set.
        assert!(!context_command::completed(context_command::global()));
        assert!(context_command::completed(0));
        assert!(
            context_command::completed(1 << 61),
            "a stale granularity is not a pending command"
        );
        assert!(!context_command::global_completed(0));
        assert!(context_command::global_completed(1 << 59));
        assert!(!context_command::global_completed(2 << 59));
        assert!(!context_command::global_completed((1 << 63) | (1 << 59)));
    }

    #[test]
    fn iotlb_command_words_and_readback() {
        assert_eq!(iotlb_command::global(), (1 << 63) | (1 << 60));
        assert_eq!(
            iotlb_command::global_with_drain(),
            (1 << 63) | (1 << 60) | (1 << 49) | (1 << 48)
        );
        assert!(!iotlb_command::completed(iotlb_command::global()));
        assert!(!iotlb_command::completed(iotlb_command::global_with_drain()));
        assert!(iotlb_command::completed(0));
        assert!(
            iotlb_command::completed(1 << 60),
            "a stale granularity is not a pending command"
        );
        assert!(!iotlb_command::global_completed(0));
        assert!(iotlb_command::global_completed(1 << 57));
        assert!(!iotlb_command::global_completed(2 << 57));
        assert!(!iotlb_command::global_completed((1 << 63) | (1 << 57)));
        assert_eq!(iotlb_command::COMMAND_REGISTER, 0x08);
    }

    #[test]
    fn fault_status_reads_each_field() {
        assert_eq!(FaultStatus::PFO, 1 << 0);
        assert_eq!(FaultStatus::PPF, 1 << 1);
        assert_eq!(FaultStatus::FRI_SHIFT, 8);
        assert_eq!(FaultStatus::FRI_MASK, 0xFF);

        let status = FaultStatus::from_bits(1 | (1 << 1) | (0x2A << 8));
        assert!(status.overflow());
        assert!(status.primary_pending());
        assert_eq!(status.fault_record_index(), 0x2A);

        // Overflow and pending are distinct bits.
        assert!(FaultStatus::from_bits(1).overflow());
        assert!(!FaultStatus::from_bits(1).primary_pending());
        assert!(!FaultStatus::from_bits(2).overflow());
        assert!(FaultStatus::from_bits(2).primary_pending());
        assert_eq!(FaultStatus::from_bits(0xFF << 8).fault_record_index(), 0xFF);
        // Acknowledging an overflow writes the one write-1-to-clear bit.
        assert_eq!(FaultStatus::acknowledge_overflow(), 1);
    }

    #[test]
    fn fault_recording_positions_match_the_spec() {
        // Positions restated relative to the 128-bit record: F bit 127, T bit
        // 126, FR bits 111:104, SID bits 79:64, FI bits 63:12.
        assert_eq!(FaultRecording::FAULT, 1 << (127 - 64));
        assert_eq!(FaultRecording::TYPE, 1 << (126 - 64));
        assert_eq!(FaultRecording::REASON_SHIFT, 104 - 64);
        assert_eq!(FaultRecording::REASON_MASK, 0xFF, "FR bits 111:104");
        assert_eq!(FaultRecording::SID_MASK, 0xFFFF, "SID bits 79:64");
    }

    #[test]
    fn a_recorded_fault_decodes_its_source_reason_and_address() {
        let record = FaultRecording {
            lo: 0xDEAD_E000,
            hi: (1 << 63) | (1 << 62) | (0x0C << 40) | 0xBEEF,
        };
        assert!(record.is_fault());
        assert_eq!(record.source_id(), 0xBEEF);
        assert_eq!(record.fault_reason(), 0x0C);
        assert_eq!(record.fault_address(), 0xDEAD_E000);
        assert!(record.is_read_request());

        // The address is masked to a page, and a clear T bit is a write.
        let unaligned = FaultRecording {
            lo: 0xDEAD_EFFF,
            hi: 1 << 63,
        };
        assert_eq!(unaligned.fault_address(), 0xDEAD_E000);
        assert!(!unaligned.is_read_request());
    }

    #[test]
    fn a_register_without_the_fault_bit_records_no_fault() {
        let clean = FaultRecording {
            lo: 0xDEAD_E000,
            hi: (0x0C << 40) | 0xBEEF, // reason and SID set, but F clear
        };
        assert!(!clean.is_fault());

        // The clear value writes 1 to F and nothing else.
        let clear = FaultRecording::clear();
        assert_eq!(clear.hi, 1 << 63);
        assert_eq!(clear.lo, 0);
    }
}
