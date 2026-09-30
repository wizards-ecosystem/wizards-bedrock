//! Intel VT-d (DMA remapping) register, table and invalidation encoding.
//!
//! Pure arithmetic over the register window and the kernel-owned root, context
//! and second-level tables. No hardware behind it: `wost`'s VT-d unit maps the
//! register window as MMIO and writes the frames this crate encodes, on the
//! `x86-paging`/`frame-space` precedent (D-30, D-43) that puts checkable bit
//! layout where `cargo test` can reach it.
//!
//! Transcribed from the Intel Virtualization Technology for Directed I/O
//! Architecture Specification, rev 3.x/4.x (the Capability, Extended
//! Capability, Global Command/Status, Context Command, IOTLB, Fault Status and
//! Fault Recording registers; the root/context/second-level table entries; and
//! the register-based and queued invalidation forms). Every field's shift and
//! mask is a named constant, and the tests restate each bit position from the
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
//!
//! // Root and context entries for domain 7, and a writable second-level leaf.
//! let root = RootEntry::present(0x1000);
//! assert_eq!(root.context_table_address(), 0x1000);
//! let context = ContextEntry::present(0x2000, width, 7);
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
pub const VER: u64 = 0x00;
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
/// Fault Event Data register (32-bit).
pub const FEDATA: u64 = 0x3C;
/// Fault Event Address register (32-bit).
pub const FEADDR: u64 = 0x40;

/// Invalidation Queue Head register (64-bit).
pub const IQH: u64 = 0x80;
/// Invalidation Queue Tail register (64-bit).
pub const IQT: u64 = 0x88;
/// Invalidation Queue Address register (64-bit).
pub const IQA: u64 = 0x90;

/// Encode the Invalidation Queue Address register value.
///
/// Bits 63:12 hold the 4-KiB-aligned queue base; bits 2:0 hold the queue size
/// field (`size_field`), so the queue spans `2^size_field` pages, i.e.
/// `2^(size_field + 8)` 128-bit descriptors. Bit 11 (descriptor width) is left
/// clear, selecting the 128-bit descriptors this crate encodes.
pub const fn iqa_value(queue_paddr: u64, size_field: u8) -> u64 {
    (queue_paddr & ADDRESS_MASK) | ((size_field as u64) & 0x7)
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

    /// Byte offset of the IOTLB register block: IRO times sixteen.
    ///
    /// Within that block the Invalidate Address register (IVA_REG) sits at
    /// [`iotlb_command::ADDRESS_REGISTER`] and the IOTLB Invalidate register
    /// (IOTLB_REG) at [`iotlb_command::COMMAND_REGISTER`].
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
/// bit while leaving the other enable bits in their intended state, which is
/// what the [`set_root_table_pointer`], [`enable_translation`] and
/// [`enable_queued_invalidation`] helpers do.
pub mod gcmd {
    /// Translation Enable, bit 31.
    pub const TE: u32 = 1 << 31;
    /// Set Root Table Pointer, bit 30.
    pub const SRTP: u32 = 1 << 30;
    /// Write Buffer Flush, bit 27.
    pub const WBF: u32 = 1 << 27;
    /// Queued Invalidation Enable, bit 26.
    pub const QIE: u32 = 1 << 26;
    /// Set Interrupt Remap Table Pointer, bit 24.
    pub const SIRTP: u32 = 1 << 24;
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
    /// Interrupt Remap Table Pointer Status, bit 24.
    pub const IRTPS: u32 = 1 << 24;
}

/// OR the Set-Root-Table-Pointer bit into a preserved command word.
pub const fn set_root_table_pointer(cmd: u32) -> u32 {
    cmd | gcmd::SRTP
}

/// OR the Translation-Enable bit into a preserved command word.
pub const fn enable_translation(cmd: u32) -> u32 {
    cmd | gcmd::TE
}

/// OR the Queued-Invalidation-Enable bit into a preserved command word.
pub const fn enable_queued_invalidation(cmd: u32) -> u32 {
    cmd | gcmd::QIE
}

/// Whether the root table pointer has been accepted (RTPS).
pub const fn root_table_pointer_set(status: u32) -> bool {
    (status & gsts::RTPS) != 0
}

/// Whether DMA translation is enabled (TES).
pub const fn translation_enabled(status: u32) -> bool {
    (status & gsts::TES) != 0
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

    /// The context-entry AW field encoding: 1, 2 or 3.
    pub const fn context_encoding(self) -> u64 {
        match self {
            Self::Level3 => 1,
            Self::Level4 => 2,
            Self::Level5 => 3,
        }
    }

    /// Choose the widest supported width whose coverage fits the MGAW.
    ///
    /// Preference is largest first (Level5, then Level4, then Level3): a wider
    /// table maps more of the address space with the same walk depth. A width
    /// is eligible only when its SAGAW bit is set *and* its addressable width
    /// does not exceed [`Capability::max_guest_address_width`], since the unit
    /// cannot translate an address wider than the MGAW. The 2-level width is
    /// never chosen; it is not modelled here.
    pub const fn choose(cap: Capability) -> Option<Self> {
        let mgaw = cap.max_guest_address_width();
        if cap.sagaw_supports(5) && Self::Level5.address_bits() <= mgaw {
            return Some(Self::Level5);
        }
        if cap.sagaw_supports(4) && Self::Level4.address_bits() <= mgaw {
            return Some(Self::Level4);
        }
        if cap.sagaw_supports(3) && Self::Level3.address_bits() <= mgaw {
            return Some(Self::Level3);
        }
        None
    }

    /// The 9-bit index `iova` selects at paging-structure `level`.
    ///
    /// Level 0 is the leaf page table; each level up is nine bits higher. The
    /// split points (12, 21, 30, 39, 48) are the standard x86-style ones and do
    /// not depend on the width — the width only bounds how many levels a walk
    /// uses, so a caller iterates `0..self.levels()`.
    pub const fn index(self, iova: u64, level: u8) -> u16 {
        let shift = 12 + 9 * (level as u32);
        ((iova >> shift) & 0x1FF) as u16
    }
}

/// The 12-bit offset within a 4-KiB page.
pub const fn page_offset(iova: u64) -> u64 {
    iova & 0xFFF
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

    /// An all-zero, not-present root entry.
    pub const fn not_present() -> Self {
        Self { lo: 0, hi: 0 }
    }

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

    /// An all-zero, not-present context entry.
    pub const fn not_present() -> Self {
        Self { lo: 0, hi: 0 }
    }

    /// A present context entry for legacy second-level-only translation.
    ///
    /// The translation-type field (bits 3:2 of the low qword) is left zero,
    /// which selects second-level-only translation. `second_level_root_paddr`
    /// must be 4-KiB aligned; its low twelve bits are masked off. Use
    /// [`try_present`](Self::try_present) to reject a misaligned pointer.
    #[must_use]
    pub const fn present(
        second_level_root_paddr: u64,
        aw: AddressWidth,
        domain_id: u16,
    ) -> Self {
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
/// share that layout, so an intermediate [`table_pointer`] is `R | W | addr`.
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
    /// Domain-granularity request (CIRG = 2).
    const DOMAIN: u64 = 2 << CIRG_SHIFT;
    /// Context Actual Invalidation Granularity, bits 60:59.
    const CAIG_SHIFT: u32 = 59;

    /// A global context-cache invalidation command word.
    pub const fn global() -> u64 {
        ICC | GLOBAL
    }

    /// A domain-selective context-cache invalidation command word.
    pub const fn domain(domain_id: u16) -> u64 {
        ICC | DOMAIN | (domain_id as u64)
    }

    /// Whether a read-back value shows the invalidation complete (ICC clear).
    pub const fn completed(value: u64) -> bool {
        (value & ICC) == 0
    }

    /// The actual granularity the unit reported (CAIG), bits 60:59.
    pub const fn actual_granularity(value: u64) -> u8 {
        ((value >> CAIG_SHIFT) & 0x3) as u8
    }
}

/// IOTLB invalidation through the IOTLB Invalidate register (IOTLB_REG).
///
/// The register sits at [`COMMAND_REGISTER`] within the IOTLB register block
/// that [`ExtendedCapability::iotlb_register_offset_bytes`] locates; the
/// Invalidate Address register (IVA_REG) sits at [`ADDRESS_REGISTER`].
pub mod iotlb_command {
    /// Offset of the Invalidate Address register (IVA_REG) within the block.
    pub const ADDRESS_REGISTER: u64 = 0x00;
    /// Offset of the IOTLB Invalidate register (IOTLB_REG) within the block.
    pub const COMMAND_REGISTER: u64 = 0x08;

    /// Drain Reads, bit 49: complete in-flight read requests before reporting.
    pub const DRAIN_READ: u64 = 1 << 49;
    /// Drain Writes, bit 48: complete in-flight write requests before reporting.
    pub const DRAIN_WRITE: u64 = 1 << 48;

    /// Invalidate IOTLB, bit 63 (set by software, cleared when done).
    const IVT: u64 = 1 << 63;
    /// IOTLB Invalidation Request Granularity, bits 61:60.
    const IIRG_SHIFT: u32 = 60;
    /// Global-granularity request (IIRG = 1).
    const GLOBAL: u64 = 1 << IIRG_SHIFT;
    /// Domain-granularity request (IIRG = 2).
    const DOMAIN: u64 = 2 << IIRG_SHIFT;
    /// IOTLB Actual Invalidation Granularity, bits 58:57.
    const IAIG_SHIFT: u32 = 57;
    /// Domain identifier, bits 47:32.
    const DID_SHIFT: u32 = 32;

    /// A global IOTLB invalidation command word.
    pub const fn global() -> u64 {
        IVT | GLOBAL
    }

    /// A domain-selective IOTLB invalidation command word.
    pub const fn domain(domain_id: u16) -> u64 {
        IVT | DOMAIN | ((domain_id as u64) << DID_SHIFT)
    }

    /// Whether a read-back value shows the invalidation complete (IVT clear).
    pub const fn completed(value: u64) -> bool {
        (value & IVT) == 0
    }

    /// The actual granularity the unit reported (IAIG), bits 58:57.
    pub const fn actual_granularity(value: u64) -> u8 {
        ((value >> IAIG_SHIFT) & 0x3) as u8
    }
}

// ---------------------------------------------------------------------------
// Queued invalidation: 128-bit descriptors.
// ---------------------------------------------------------------------------

/// Queued-invalidation descriptors, each a 128-bit `[low, high]` pair.
///
/// The descriptor type is bits 3:0 of the low qword; the invalidation
/// granularity is bits 5:4 (1 = global, 2 = domain-selective); a domain
/// identifier, when present, is bits 31:16.
pub mod queued {
    /// Drain Reads for an IOTLB descriptor, bit 6.
    pub const IOTLB_DRAIN_READ: u64 = 1 << 6;
    /// Drain Writes for an IOTLB descriptor, bit 7.
    pub const IOTLB_DRAIN_WRITE: u64 = 1 << 7;

    /// Context-cache invalidate descriptor type.
    const CC_TYPE: u64 = 0x1;
    /// IOTLB invalidate descriptor type.
    const IOTLB_TYPE: u64 = 0x2;
    /// Invalidation-wait descriptor type.
    const WAIT_TYPE: u64 = 0x5;
    /// Granularity field, bits 5:4.
    const GRAN_SHIFT: u32 = 4;
    /// Global granularity (field = 1).
    const GRAN_GLOBAL: u64 = 1 << GRAN_SHIFT;
    /// Domain granularity (field = 2).
    const GRAN_DOMAIN: u64 = 2 << GRAN_SHIFT;
    /// Domain identifier, bits 31:16.
    const DID_SHIFT: u32 = 16;
    /// Status Write, bit 5 of a wait descriptor.
    const WAIT_SW: u64 = 1 << 5;
    /// Status data, bits 63:32 of a wait descriptor's low qword.
    const WAIT_DATA_SHIFT: u32 = 32;
    /// Status address, bits 63:2 of a wait descriptor's high qword.
    const STATUS_ADDRESS_MASK: u64 = !0b11;

    /// A global context-cache invalidate descriptor.
    pub const fn context_cache_inval_descriptor_global() -> [u64; 2] {
        [CC_TYPE | GRAN_GLOBAL, 0]
    }

    /// A domain-selective context-cache invalidate descriptor.
    pub const fn context_cache_inval_descriptor_domain(domain_id: u16) -> [u64; 2] {
        [CC_TYPE | GRAN_DOMAIN | ((domain_id as u64) << DID_SHIFT), 0]
    }

    /// A global IOTLB invalidate descriptor.
    pub const fn iotlb_inval_descriptor_global() -> [u64; 2] {
        [IOTLB_TYPE | GRAN_GLOBAL, 0]
    }

    /// A domain-selective IOTLB invalidate descriptor.
    pub const fn iotlb_inval_descriptor_domain(domain_id: u16) -> [u64; 2] {
        [IOTLB_TYPE | GRAN_DOMAIN | ((domain_id as u64) << DID_SHIFT), 0]
    }

    /// An invalidation-wait descriptor that writes `status_data` to
    /// `status_write_paddr` once earlier descriptors have completed.
    ///
    /// The low qword carries the type (bits 3:0 = 5), the Status Write flag
    /// (bit 5), and `status_data` in bits 63:32. The high qword carries the
    /// status address in bits 63:2; its low two bits are masked off, so the
    /// address is 4-byte aligned.
    pub const fn invalidation_wait_descriptor(
        status_write_paddr: u64,
        status_data: u32,
    ) -> [u64; 2] {
        let low = WAIT_TYPE | WAIT_SW | ((status_data as u64) << WAIT_DATA_SHIFT);
        let high = status_write_paddr & STATUS_ADDRESS_MASK;
        [low, high]
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

    /// The raw register value.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether a primary fault is pending (PPF).
    pub const fn primary_pending(self) -> bool {
        (self.0 & Self::PPF) != 0
    }

    /// Whether the fault log overflowed (PFO).
    pub const fn overflow(self) -> bool {
        (self.0 & Self::PFO) != 0
    }

    /// Index of the fault-recording register holding the first pending fault.
    pub const fn fault_record_index(self) -> u8 {
        ((self.0 >> Self::FRI_SHIFT) & Self::FRI_MASK) as u8
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
    /// set bit as a read request. Some specification revisions also define an
    /// address-type field in the neighbouring bits, which is not decoded here.
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
    fn register_offsets_are_the_spec_byte_offsets() {
        assert_eq!(VER, 0x00);
        assert_eq!(CAP, 0x08);
        assert_eq!(ECAP, 0x10);
        assert_eq!(GCMD, 0x18);
        assert_eq!(GSTS, 0x1C);
        assert_eq!(RTADDR, 0x20);
        assert_eq!(CCMD, 0x28);
        assert_eq!(FSTS, 0x34);
        assert_eq!(FECTL, 0x38);
        assert_eq!(FEDATA, 0x3C);
        assert_eq!(FEADDR, 0x40);
        assert_eq!(IQH, 0x80);
        assert_eq!(IQT, 0x88);
        assert_eq!(IQA, 0x90);
    }

    /// The address field is bits 63:12 (52 bits), independent of the constant.
    #[test]
    fn the_entry_address_field_is_bits_63_through_12() {
        assert_eq!(ADDRESS_MASK, !0xFFFu64);
        assert_eq!(ADDRESS_MASK.trailing_zeros(), 12);
        assert_eq!(ADDRESS_MASK.count_ones(), 52);
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
    }

    #[test]
    fn capability_decodes_each_field_from_its_own_bits() {
        // Pairwise-distinct field values placed with independent literal shifts.
        let cap = Capability::from_bits(
            5 | (1 << 4) | (1 << 7) | (4 << 8) | (47 << 16) | (0x123u64 << 24) | (0x12u64 << 40),
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
        assert_eq!(ExtendedCapability::IRO_SHIFT, 8);
        assert_eq!(ExtendedCapability::IRO_MASK, 0x3FF, "IRO bits 17:8");
    }

    #[test]
    fn extended_capability_decodes_each_field() {
        let ecap = ExtendedCapability::from_bits(1 | (1 << 1) | (1 << 3) | (0x2Au64 << 8));
        assert!(ecap.page_walk_coherent());
        assert!(ecap.queued_invalidation());
        assert!(ecap.interrupt_remapping());
        assert_eq!(ecap.iotlb_register_offset_bytes(), 0x2A * 16);

        // Only IRO set: the three single-bit fields read false.
        let only_iro = ExtendedCapability::from_bits(0x2Au64 << 8);
        assert!(!only_iro.page_walk_coherent());
        assert!(!only_iro.queued_invalidation());
        assert!(!only_iro.interrupt_remapping());
        assert_eq!(only_iro.iotlb_register_offset_bytes(), 0x2A * 16);
        assert_eq!(only_iro.bits(), 0x2Au64 << 8);
    }

    #[test]
    fn global_command_and_status_bits_match_the_spec() {
        assert_eq!(gcmd::TE, 1 << 31);
        assert_eq!(gcmd::SRTP, 1 << 30);
        assert_eq!(gcmd::WBF, 1 << 27);
        assert_eq!(gcmd::QIE, 1 << 26);
        assert_eq!(gcmd::SIRTP, 1 << 24);
        assert_eq!(gsts::TES, 1 << 31);
        assert_eq!(gsts::RTPS, 1 << 30);
        assert_eq!(gsts::WBFS, 1 << 27);
        assert_eq!(gsts::QIES, 1 << 26);
        assert_eq!(gsts::IRTPS, 1 << 24);
    }

    #[test]
    fn command_helpers_or_one_bit_into_a_preserved_word() {
        let preserved = gcmd::QIE | gcmd::TE;
        assert_eq!(set_root_table_pointer(preserved), preserved | gcmd::SRTP);
        assert_eq!(enable_translation(0), gcmd::TE);
        assert_eq!(enable_queued_invalidation(0), gcmd::QIE);
        // The preserved bits survive.
        assert_eq!(enable_translation(gcmd::QIE) & gcmd::QIE, gcmd::QIE);
    }

    #[test]
    fn status_predicates_read_their_own_bit() {
        assert!(root_table_pointer_set(gsts::RTPS));
        assert!(!root_table_pointer_set(gsts::TES));
        assert!(translation_enabled(gsts::TES));
        assert!(!translation_enabled(gsts::RTPS));
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

    #[test]
    fn choose_prefers_the_widest_supported_within_the_mgaw() {
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
    fn choose_drops_a_width_that_exceeds_the_mgaw() {
        // 4-level supported, MGAW only 39 bits: 48 > 39, and 3-level is not
        // supported, so nothing fits.
        let cap = Capability::from_bits((1 << 10) | (38u64 << 16));
        assert_eq!(AddressWidth::choose(cap), None);

        // Both 3- and 4-level supported; MGAW 48 admits 4-level.
        let mgaw48 = Capability::from_bits((1 << 9) | (1 << 10) | (47u64 << 16));
        assert_eq!(AddressWidth::choose(mgaw48), Some(AddressWidth::Level4));

        // MGAW 47 rejects 4-level (48) and falls back to 3-level (39).
        let mgaw47 = Capability::from_bits((1 << 9) | (1 << 10) | (46u64 << 16));
        assert_eq!(AddressWidth::choose(mgaw47), Some(AddressWidth::Level3));
    }

    #[test]
    fn index_splits_an_iova_nine_bits_at_a_time() {
        // Distinct multi-bit indices placed with independent literal shifts.
        let iova = 0xABC
            | (0x1F << 12)
            | (0x2E << 21)
            | (0x3D << 30)
            | (0x4Cu64 << 39)
            | (0x5Bu64 << 48);
        let w = AddressWidth::Level5;
        assert_eq!(w.index(iova, 0), 0x1F);
        assert_eq!(w.index(iova, 1), 0x2E);
        assert_eq!(w.index(iova, 2), 0x3D);
        assert_eq!(w.index(iova, 3), 0x4C);
        assert_eq!(w.index(iova, 4), 0x5B);
        assert_eq!(page_offset(iova), 0xABC);
    }

    #[test]
    fn page_offset_is_the_low_twelve_bits() {
        assert_eq!(page_offset(0xFFF), 0xFFF);
        assert_eq!(page_offset(0x1000), 0);
        assert_eq!(page_offset(0x1234_5678), 0x678);
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
        assert_eq!(masked.lo & 0xFFE, 0, "no reserved low bit but present is set");
        assert!(masked.is_present());
    }

    #[test]
    fn a_not_present_root_entry_is_all_zero() {
        let entry = RootEntry::not_present();
        assert_eq!((entry.lo, entry.hi), (0, 0));
        assert!(!entry.is_present());
        assert_eq!(RootEntry::default(), entry);
    }

    #[test]
    fn root_entry_alignment_is_checked_only_by_try_present() {
        assert!(RootEntry::try_present(0x2000).is_some());
        assert_eq!(RootEntry::try_present(0x2001), None);
        assert_eq!(RootEntry::try_present(0x2000), Some(RootEntry::present(0x2000)));
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

        let none = ContextEntry::not_present();
        assert!(!none.is_present());
        assert_eq!((none.lo, none.hi), (0, 0));
        assert_eq!(
            ContextEntry::try_present(0x3001, AddressWidth::Level4, 0),
            None
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

    #[test]
    fn context_command_words_and_readback() {
        assert_eq!(context_command::global(), (1 << 63) | (1 << 61));
        assert_eq!(
            context_command::domain(0x1234),
            (1 << 63) | (2 << 61) | 0x1234
        );
        // ICC clear means complete; a fresh command has ICC set.
        assert!(!context_command::completed(context_command::global()));
        assert!(context_command::completed(0));
        // CAIG is bits 60:59.
        assert_eq!(context_command::actual_granularity(2 << 59), 2);
        assert_eq!(context_command::actual_granularity(1 << 59), 1);
    }

    #[test]
    fn iotlb_command_words_and_readback() {
        assert_eq!(iotlb_command::global(), (1 << 63) | (1 << 60));
        assert_eq!(
            iotlb_command::domain(0x5678),
            (1 << 63) | (2 << 60) | (0x5678u64 << 32)
        );
        assert!(!iotlb_command::completed(iotlb_command::global()));
        assert!(iotlb_command::completed(0));
        // IAIG is bits 58:57.
        assert_eq!(iotlb_command::actual_granularity(2 << 57), 2);
        assert_eq!(iotlb_command::actual_granularity(1 << 57), 1);
        // Drain bits and register offsets.
        assert_eq!(iotlb_command::DRAIN_READ, 1 << 49);
        assert_eq!(iotlb_command::DRAIN_WRITE, 1 << 48);
        assert_eq!(iotlb_command::ADDRESS_REGISTER, 0x00);
        assert_eq!(iotlb_command::COMMAND_REGISTER, 0x08);
    }

    #[test]
    fn queued_context_cache_descriptors_carry_type_and_granularity() {
        assert_eq!(
            queued::context_cache_inval_descriptor_global(),
            [0x1 | (1 << 4), 0]
        );
        assert_eq!(
            queued::context_cache_inval_descriptor_domain(0x9ABC),
            [0x1 | (2 << 4) | (0x9ABCu64 << 16), 0]
        );
    }

    #[test]
    fn queued_iotlb_descriptors_carry_type_and_granularity() {
        assert_eq!(queued::iotlb_inval_descriptor_global(), [0x2 | (1 << 4), 0]);
        assert_eq!(
            queued::iotlb_inval_descriptor_domain(0x9ABC),
            [0x2 | (2 << 4) | (0x9ABCu64 << 16), 0]
        );
        assert_eq!(queued::IOTLB_DRAIN_READ, 1 << 6);
        assert_eq!(queued::IOTLB_DRAIN_WRITE, 1 << 7);
    }

    #[test]
    fn a_wait_descriptor_places_data_and_a_masked_address() {
        let descriptor = queued::invalidation_wait_descriptor(0xDEAD_B000, 0xCAFE_F00D);
        // low: type 5 | SW (bit 5) | data in bits 63:32.
        assert_eq!(descriptor[0], 0x5 | (1 << 5) | (0xCAFE_F00Du64 << 32));
        // high: 4-byte-aligned status address in bits 63:2.
        assert_eq!(descriptor[1], 0xDEAD_B000);

        // The address's low two bits are masked off.
        let unaligned = queued::invalidation_wait_descriptor(0xDEAD_B003, 0);
        assert_eq!(unaligned[1], 0xDEAD_B000);
        assert_eq!(unaligned[0], 0x5 | (1 << 5));
    }

    #[test]
    fn the_invalidation_queue_address_carries_size_and_a_masked_base() {
        assert_eq!(iqa_value(0x7654_3000, 3), 0x7654_3000 | 3);
        // The size field is three bits.
        assert_eq!(iqa_value(0x1000, 0xFF), 0x1000 | 7);
        // The base is masked to 4 KiB, and bit 11 stays clear.
        assert_eq!(iqa_value(0x1FFF, 0), 0x1000);
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
        assert_eq!(status.bits(), 1 | (1 << 1) | (0x2A << 8));

        // Overflow and pending are distinct bits.
        assert!(FaultStatus::from_bits(1).overflow());
        assert!(!FaultStatus::from_bits(1).primary_pending());
        assert!(!FaultStatus::from_bits(2).overflow());
        assert!(FaultStatus::from_bits(2).primary_pending());
        assert_eq!(FaultStatus::from_bits(0xFF << 8).fault_record_index(), 0xFF);
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

        // The address is masked to a page.
        let unaligned = FaultRecording {
            lo: 0xDEAD_EFFF,
            hi: 1 << 63,
        };
        assert_eq!(unaligned.fault_address(), 0xDEAD_E000);
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
