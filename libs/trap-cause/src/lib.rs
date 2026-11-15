#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;
use core::ops::Range;

use x86_tables::exception;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrapCause {
    Fault(Fault),
    Reserved(u8),
    Interrupt(u8),
}

impl TrapCause {
    pub const fn decode(vector: u8, error_code: u64) -> Self {
        if vector >= exception::COUNT {
            return Self::Interrupt(vector);
        }
        match vector {
            0 => Self::Fault(Fault::DivideError),
            1 => Self::Fault(Fault::Debug),
            2 => Self::Fault(Fault::NonMaskable),
            3 => Self::Fault(Fault::Breakpoint),
            4 => Self::Fault(Fault::Overflow),
            5 => Self::Fault(Fault::BoundRange),
            6 => Self::Fault(Fault::InvalidOpcode),
            7 => Self::Fault(Fault::DeviceNotAvailable),
            8 => Self::Fault(Fault::DoubleFault),
            10 => Self::Fault(Fault::InvalidTaskState),
            11 => Self::Fault(Fault::SegmentNotPresent),
            12 => Self::Fault(Fault::StackSegment),
            13 => Self::Fault(Fault::GeneralProtection),
            14 => Self::Fault(Fault::PageFault(PageFault(error_code))),
            16 => Self::Fault(Fault::X87FloatingPoint),
            17 => Self::Fault(Fault::AlignmentCheck),
            18 => Self::Fault(Fault::MachineCheck),
            19 => Self::Fault(Fault::SimdFloatingPoint),
            20 => Self::Fault(Fault::Virtualisation),
            21 => Self::Fault(Fault::ControlProtection),
            28 => Self::Fault(Fault::HypervisorInjection),
            29 => Self::Fault(Fault::VmmCommunication),
            30 => Self::Fault(Fault::Security),
            other => Self::Reserved(other),
        }
    }

    pub const fn vector(self) -> u8 {
        match self {
            Self::Fault(fault) => fault.vector(),
            Self::Reserved(vector) | Self::Interrupt(vector) => vector,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Fault(fault) => fault.name(),
            Self::Reserved(_) => "reserved vector",
            Self::Interrupt(_) => "interrupt",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fault {
    DivideError,
    Debug,
    NonMaskable,
    Breakpoint,
    Overflow,
    BoundRange,
    InvalidOpcode,
    DeviceNotAvailable,
    DoubleFault,
    InvalidTaskState,
    SegmentNotPresent,
    StackSegment,
    GeneralProtection,
    PageFault(PageFault),
    X87FloatingPoint,
    AlignmentCheck,
    MachineCheck,
    SimdFloatingPoint,
    Virtualisation,
    ControlProtection,
    HypervisorInjection,
    VmmCommunication,
    Security,
}

impl Fault {
    pub const fn vector(self) -> u8 {
        match self {
            Self::DivideError => 0,
            Self::Debug => 1,
            Self::NonMaskable => 2,
            Self::Breakpoint => 3,
            Self::Overflow => 4,
            Self::BoundRange => 5,
            Self::InvalidOpcode => 6,
            Self::DeviceNotAvailable => 7,
            Self::DoubleFault => 8,
            Self::InvalidTaskState => 10,
            Self::SegmentNotPresent => 11,
            Self::StackSegment => 12,
            Self::GeneralProtection => 13,
            Self::PageFault(_) => 14,
            Self::X87FloatingPoint => 16,
            Self::AlignmentCheck => 17,
            Self::MachineCheck => 18,
            Self::SimdFloatingPoint => 19,
            Self::Virtualisation => 20,
            Self::ControlProtection => 21,
            Self::HypervisorInjection => 28,
            Self::VmmCommunication => 29,
            Self::Security => 30,
        }
    }

    pub const fn name(self) -> &'static str {
        match exception::name(self.vector()) {
            Some(name) => name,
            None => "exception",
        }
    }
}

// The error code bits of Intel SDM Vol. 3A §4.7.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageFault(u64);

impl PageFault {
    const fn is_supervisor_data_access(self) -> bool {
        self.0 & !0b11 == 0
    }

    const fn was_present(self) -> bool {
        self.0 & 1 != 0
    }

    const fn was_write(self) -> bool {
        self.0 & (1 << 1) != 0
    }

    pub const fn from_user(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    const fn reserved_bit_set(self) -> bool {
        self.0 & (1 << 3) != 0
    }

    // Meaningful only with NX enabled.
    const fn was_fetch(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    const fn protection_key(self) -> bool {
        self.0 & (1 << 5) != 0
    }

    const fn shadow_stack(self) -> bool {
        self.0 & (1 << 6) != 0
    }

    pub const fn bits(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for PageFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} on {} from {}",
            if self.was_present() {
                "protection violation"
            } else {
                "no mapping"
            },
            if self.was_fetch() {
                "instruction fetch"
            } else if self.was_write() {
                "write"
            } else {
                "read"
            },
            if self.from_user() { "user" } else { "kernel" }
        )?;
        if self.reserved_bit_set() {
            formatter.write_str(", reserved bit set in a table entry")?;
        }
        if self.protection_key() {
            formatter.write_str(", protection key")?;
        }
        if self.shadow_stack() {
            formatter.write_str(", shadow stack")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecoveryAttempt {
    pub vector: u64,
    pub frame_error_code: u64,
    pub code_selector: u64,
    pub instruction_pointer: u64,
    pub page_fault: PageFault,
    pub faulting_address: u64,
}

impl RecoveryAttempt {
    pub fn is_admitted_by(&self, rule: &RecoveryRule) -> bool {
        let class_matches = match rule.fault {
            RecoveryFault::SupervisorData => self.page_fault.is_supervisor_data_access(),
            RecoveryFault::SupervisorWriteProtection => self.page_fault.bits() == 0b11,
        };

        self.vector == u64::from(exception::PAGE_FAULT)
            && self.frame_error_code == self.page_fault.bits()
            && self.code_selector & 0b11 == 0
            && class_matches
            && rule.fault_addresses.contains(&self.faulting_address)
            && rule
                .instruction_addresses
                .contains(&self.instruction_pointer)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecoveryFault {
    SupervisorData,
    SupervisorWriteProtection,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecoveryRule {
    pub fault: RecoveryFault,
    pub fault_addresses: Range<u64>,
    pub instruction_addresses: Range<u64>,
}

#[cfg(test)]
mod tests;
