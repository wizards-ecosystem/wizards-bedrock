#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

const ADDRESS_MASK: u64 = 0xFFFF_FFFF_FFFF_F000;

pub mod register {
    pub const VERSION: u64 = 0x00;
    pub const CAPABILITY: u64 = 0x08;
    pub const EXTENDED_CAPABILITY: u64 = 0x10;
    pub const GLOBAL_COMMAND: u64 = 0x18;
    pub const GLOBAL_STATUS: u64 = 0x1C;
    pub const ROOT_TABLE_ADDRESS: u64 = 0x20;
    pub const CONTEXT_COMMAND: u64 = 0x28;
    pub const FAULT_STATUS: u64 = 0x34;
    pub const FAULT_EVENT_CONTROL: u64 = 0x38;
    pub const INVALIDATION_QUEUE_TAIL: u64 = 0x88;
    pub const INVALIDATION_QUEUE_ADDRESS: u64 = 0x90;
    pub const INTERRUPT_REMAPPING_TABLE_ADDRESS: u64 = 0xB8;
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Version(u32);

impl Version {
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn major(self) -> u8 {
        ((self.0 >> 4) & 0xF) as u8
    }

    pub const fn minor(self) -> u8 {
        (self.0 & 0xF) as u8
    }

    // Majors six and up ignore register invalidation; zero is malformed.
    pub const fn register_invalidation_supported(self) -> bool {
        matches!(self.major(), 1..=5)
    }
}

// Fault events bypass interrupt remapping, so the owner keeps them masked.
pub mod fault_event_control {
    pub const MASK: u32 = 1 << 31;

    pub const fn masked(control: u32) -> bool {
        control & MASK != 0
    }
}

// Bits 11:10 of RTADDR select the translation mode, left at legacy.
pub const fn root_table_address(pointer: u64) -> u64 {
    pointer & ADDRESS_MASK
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Capability(u64);

impl Capability {
    const ND_MASK: u64 = 0x7;
    const RWBF: u64 = 1 << 4;
    const CM: u64 = 1 << 7;
    const SAGAW_SHIFT: u32 = 8;
    const SAGAW_MASK: u64 = 0x1F;
    const MGAW_SHIFT: u32 = 16;
    const MGAW_MASK: u64 = 0x3F;
    const FRO_SHIFT: u32 = 24;
    const FRO_MASK: u64 = 0x3FF;
    const NFR_SHIFT: u32 = 40;
    const NFR_MASK: u64 = 0xFF;
    const DWD: u64 = 1 << 54;
    const DRD: u64 = 1 << 55;

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn number_of_domains(self) -> u32 {
        let field = (self.0 & Self::ND_MASK) as u32;
        1u32 << (4 + 2 * field)
    }

    pub const fn requires_write_buffer_flush(self) -> bool {
        (self.0 & Self::RWBF) != 0
    }

    // The unit caches not-present entries, so every such change needs invalidation.
    pub const fn caching_mode(self) -> bool {
        (self.0 & Self::CM) != 0
    }

    const fn sagaw_supports(self, levels: u8) -> bool {
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

    const fn max_guest_address_width(self) -> u8 {
        (((self.0 >> Self::MGAW_SHIFT) & Self::MGAW_MASK) as u8) + 1
    }

    pub const fn fault_recording_offset_bytes(self) -> u64 {
        ((self.0 >> Self::FRO_SHIFT) & Self::FRO_MASK) * 16
    }

    pub const fn number_of_fault_recording_registers(self) -> u16 {
        (((self.0 >> Self::NFR_SHIFT) & Self::NFR_MASK) as u16) + 1
    }

    const fn write_draining(self) -> bool {
        (self.0 & Self::DWD) != 0
    }

    const fn read_draining(self) -> bool {
        (self.0 & Self::DRD) != 0
    }

    pub const fn dma_draining(self) -> bool {
        self.write_draining() && self.read_draining()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ExtendedCapability(u64);

impl ExtendedCapability {
    const C: u64 = 1 << 0;
    const QI: u64 = 1 << 1;
    const IR: u64 = 1 << 3;
    const EIM: u64 = 1 << 4;
    const IRO_SHIFT: u32 = 8;
    const IRO_MASK: u64 = 0x3FF;

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    // When clear, each entry write needs a cache flush.
    pub const fn page_walk_coherent(self) -> bool {
        (self.0 & Self::C) != 0
    }

    pub const fn queued_invalidation(self) -> bool {
        (self.0 & Self::QI) != 0
    }

    pub const fn interrupt_remapping(self) -> bool {
        (self.0 & Self::IR) != 0
    }

    pub const fn extended_interrupt_mode(self) -> bool {
        (self.0 & Self::EIM) != 0
    }

    pub const fn iotlb_register_offset_bytes(self) -> u64 {
        ((self.0 >> Self::IRO_SHIFT) & Self::IRO_MASK) * 16
    }
}

// Each command is written over a software-preserved shadow of the enable bits.
pub mod global_command {
    pub const TE: u32 = 1 << 31;
    pub(crate) const SRTP: u32 = 1 << 30;
    pub const WBF: u32 = 1 << 27;
    pub const QIE: u32 = 1 << 26;
    pub const IRE: u32 = 1 << 25;
    pub(crate) const SIRTP: u32 = 1 << 24;
    pub const CFI: u32 = 1 << 23;
}

pub mod global_status {
    pub(crate) const TES: u32 = 1 << 31;
    pub(crate) const RTPS: u32 = 1 << 30;
    pub const WBFS: u32 = 1 << 27;
    pub const QIES: u32 = 1 << 26;
    pub(crate) const IRES: u32 = 1 << 25;
    pub(crate) const IRTPS: u32 = 1 << 24;
    pub(crate) const CFIS: u32 = 1 << 23;
}

pub const fn set_root_table_pointer(command: u32) -> u32 {
    command | global_command::SRTP
}

pub const fn enable_translation(command: u32) -> u32 {
    command | global_command::TE
}

pub const fn set_interrupt_table_pointer(command: u32) -> u32 {
    command | global_command::SIRTP
}

pub const fn enable_interrupt_remapping(command: u32) -> u32 {
    (command | global_command::IRE) & !global_command::CFI
}

pub const fn root_table_pointer_set(status: u32) -> bool {
    (status & global_status::RTPS) != 0
}

pub const fn translation_enabled(status: u32) -> bool {
    (status & global_status::TES) != 0
}

pub const fn interrupt_table_pointer_set(status: u32) -> bool {
    (status & global_status::IRTPS) != 0
}

pub const fn interrupt_remapping_enabled(status: u32) -> bool {
    (status & global_status::IRES) != 0
}

pub const fn compatibility_interrupts_enabled(status: u32) -> bool {
    (status & global_status::CFIS) != 0
}

pub const fn takeover_ready(status: u32) -> bool {
    status & (global_status::TES | global_status::QIES | global_status::IRES | global_status::CFIS)
        == 0
}

const REMAPPING_ADDRESS_MASK: u64 = 0xFFFF_FFFF_FFFF_F000;

// Only x2APIC delivery is admitted, and EIME blocks compatibility format whatever CFI says.
pub const fn interrupt_table_address(address: u64) -> Option<u64> {
    if address & !REMAPPING_ADDRESS_MASK != 0 {
        return None;
    }
    Some((address & REMAPPING_ADDRESS_MASK) | (1 << 11))
}

pub const fn interrupt_table_base(value: u64) -> u64 {
    value & REMAPPING_ADDRESS_MASK
}

pub const fn interrupt_table_has_two_entries(value: u64) -> bool {
    value & 0xF == 0
}

pub const fn invalidation_queue_address(address: u64) -> Option<u64> {
    if address & !REMAPPING_ADDRESS_MASK != 0 {
        return None;
    }
    Some(address & REMAPPING_ADDRESS_MASK)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InvalidationDescriptor {
    pub lo: u64,
    pub hi: u64,
}

impl InvalidationDescriptor {
    pub const fn global_interrupt_entry_cache() -> Self {
        Self { lo: 4, hi: 0 }
    }

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddressWidth {
    Level3,
    Level4,
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

    const fn address_bits(self) -> u8 {
        match self {
            Self::Level3 => 39,
            Self::Level4 => 48,
            Self::Level5 => 57,
        }
    }

    pub const fn covers(self, address: u64) -> bool {
        address >> self.address_bits() == 0
    }

    pub const fn context_encoding(self) -> u64 {
        match self {
            Self::Level3 => 1,
            Self::Level4 => 2,
            Self::Level5 => 3,
        }
    }

    pub const fn covers_on(self, capability: Capability, address: u64) -> bool {
        let bits = capability.max_guest_address_width();
        self.covers(address) && (bits == 64 || address >> bits == 0)
    }

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

    pub const fn index(self, iova: u64, level: u8) -> u16 {
        let shift = 12 + 9 * (level as u32);
        match iova.checked_shr(shift) {
            Some(shifted) => (shifted & 0x1FF) as u16,
            None => 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct RootEntry {
    pub lo: u64,
    pub hi: u64,
}

impl RootEntry {
    const PRESENT: u64 = 1 << 0;

    #[must_use]
    const fn present(context_table: u64) -> Self {
        Self {
            lo: (context_table & ADDRESS_MASK) | Self::PRESENT,
            hi: 0,
        }
    }

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

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ContextEntry {
    pub lo: u64,
    pub hi: u64,
}

impl ContextEntry {
    const PRESENT: u64 = 1 << 0;
    const AW_MASK: u64 = 0x7;
    const DID_SHIFT: u32 = 8;

    #[must_use]
    const fn present(second_level_root: u64, width: AddressWidth, domain_id: u16) -> Self {
        Self {
            lo: (second_level_root & ADDRESS_MASK) | Self::PRESENT,
            hi: width.context_encoding() | ((domain_id as u64) << Self::DID_SHIFT),
        }
    }

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

    pub const fn translation_type(self) -> u64 {
        (self.lo >> 2) & 0x3
    }

    pub const fn second_level_address(self) -> u64 {
        self.lo & ADDRESS_MASK
    }

    pub const fn address_width_encoding(self) -> u64 {
        self.hi & Self::AW_MASK
    }

    pub const fn domain_id(self) -> u16 {
        ((self.hi >> Self::DID_SHIFT) & 0xFFFF) as u16
    }
}

pub mod second_level {
    use super::ADDRESS_MASK;

    const READ: u64 = 1 << 0;
    const WRITE: u64 = 1 << 1;

    pub const NOT_PRESENT: u64 = 0;

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

    pub const fn table_pointer(physical: u64) -> u64 {
        (physical & ADDRESS_MASK) | READ | WRITE
    }

    pub const fn is_present(entry: u64) -> bool {
        entry & (READ | WRITE) != 0
    }

    pub const fn address(entry: u64) -> u64 {
        entry & ADDRESS_MASK
    }
}

// CCMD is reserved while queued invalidation is enabled.
pub mod context_command {
    const ICC: u64 = 1 << 63;
    const CIRG_SHIFT: u32 = 61;
    const GLOBAL: u64 = 1 << CIRG_SHIFT;
    const CAIG_SHIFT: u32 = 59;
    const CAIG_MASK: u64 = 0b11;
    const ACTUAL_GLOBAL: u64 = 0b01;

    pub const fn global() -> u64 {
        ICC | GLOBAL
    }

    const fn completed(value: u64) -> bool {
        (value & ICC) == 0
    }

    pub const fn global_completed(value: u64) -> bool {
        completed(value) && ((value >> CAIG_SHIFT) & CAIG_MASK) == ACTUAL_GLOBAL
    }
}

// The IOTLB register is reserved while queued invalidation is enabled.
pub mod iotlb_command {
    pub const COMMAND: u64 = 0x08;

    const IVT: u64 = 1 << 63;
    const IIRG_SHIFT: u32 = 60;
    const GLOBAL: u64 = 1 << IIRG_SHIFT;
    const DRAIN_READS: u64 = 1 << 49;
    const DRAIN_WRITES: u64 = 1 << 48;
    const IAIG_SHIFT: u32 = 57;
    const IAIG_MASK: u64 = 0b11;
    const ACTUAL_GLOBAL: u64 = 0b01;

    pub const fn global() -> u64 {
        IVT | GLOBAL
    }

    // The caller must first require DRD and DWD.
    pub const fn global_with_drain() -> u64 {
        global() | DRAIN_READS | DRAIN_WRITES
    }

    const fn completed(value: u64) -> bool {
        (value & IVT) == 0
    }

    pub const fn global_completed(value: u64) -> bool {
        completed(value) && ((value >> IAIG_SHIFT) & IAIG_MASK) == ACTUAL_GLOBAL
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct FaultStatus(u32);

impl FaultStatus {
    const PFO: u32 = 1 << 0;
    const PPF: u32 = 1 << 1;
    const FRI_SHIFT: u32 = 8;
    const FRI_MASK: u32 = 0xFF;

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn primary_pending(self) -> bool {
        (self.0 & Self::PPF) != 0
    }

    // An overflow stops recording until it is acknowledged.
    pub const fn overflow(self) -> bool {
        (self.0 & Self::PFO) != 0
    }

    pub const fn fault_record_index(self) -> u8 {
        ((self.0 >> Self::FRI_SHIFT) & Self::FRI_MASK) as u8
    }

    // PFO is write-one-to-clear.
    pub const fn acknowledge_overflow() -> u32 {
        Self::PFO
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct FaultRecording {
    pub lo: u64,
    pub hi: u64,
}

impl FaultRecording {
    const FAULT: u64 = 1 << 63;
    const TYPE: u64 = 1 << 62;
    const REASON_SHIFT: u32 = 32;
    const REASON_MASK: u64 = 0xFF;
    const SID_MASK: u64 = 0xFFFF;

    pub const fn is_fault(self) -> bool {
        (self.hi & Self::FAULT) != 0
    }

    pub const fn source_id(self) -> u16 {
        (self.hi & Self::SID_MASK) as u16
    }

    pub const fn fault_reason(self) -> u8 {
        ((self.hi >> Self::REASON_SHIFT) & Self::REASON_MASK) as u8
    }

    pub const fn fault_address(self) -> u64 {
        self.lo & ADDRESS_MASK
    }

    // Clear means a device write; AT is not decoded.
    pub const fn is_read_request(self) -> bool {
        (self.hi & Self::TYPE) != 0
    }

    // Bit 127 is write-one-to-clear; the other fields are read-only.
    pub const fn clear() -> Self {
        Self {
            lo: 0,
            hi: Self::FAULT,
        }
    }
}
