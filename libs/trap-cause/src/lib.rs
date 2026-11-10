//! What a trap was, decoded once from the vector and error code the CPU pushed.
//!
//! # Examples
//!
//! ```
//! use trap_cause::{Fault, TrapCause};
//!
//! // A write to a page that was present is a protection violation.
//! let cause = TrapCause::decode(14, 0b11);
//! let TrapCause::Fault(Fault::PageFault(fault)) = cause else {
//!     panic!("vector 14 is a page fault");
//! };
//! assert!(fault.was_present());
//! assert!(fault.was_write());
//! assert!(!fault.from_user());
//!
//! // Anything at or above the architectural range is a device's line.
//! assert_eq!(TrapCause::decode(0x42, 0), TrapCause::Interrupt(0x42));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;
use core::ops::Range;

use x86_tables::exception;

/// Why the CPU entered the kernel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum TrapCause {
    /// An architectural exception the CPU raised.
    Fault(Fault),
    /// A vector the architecture reserves, a statement about the machine and not a device.
    Reserved(u8),
    /// A device or software interrupt, by vector.
    Interrupt(u8),
}

impl TrapCause {
    /// Decode `vector` and its error code, which is ignored for vectors that push none.
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

    /// The vector this was decoded from.
    pub const fn vector(self) -> u8 {
        match self {
            Self::Fault(fault) => fault.vector(),
            Self::Reserved(vector) | Self::Interrupt(vector) => vector,
        }
    }

    /// What to call this in a report.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fault(fault) => fault.name(),
            Self::Reserved(_) => "reserved vector",
            Self::Interrupt(_) => "interrupt",
        }
    }
}

/// An architectural exception.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Fault {
    /// `#DE`: a division whose result does not fit, or by zero.
    DivideError,
    /// `#DB`: a hardware breakpoint or a single step.
    Debug,
    /// Not masked by the interrupt flag, so no lock excludes it.
    NonMaskable,
    /// `#BP`: the trap `int3` plants, the one resumable entry here.
    Breakpoint,
    /// `#OF`: `into` with the overflow flag set.
    Overflow,
    /// `#BR`: a `bound` check that failed.
    BoundRange,
    /// `#UD`: an opcode the CPU does not implement in this mode.
    InvalidOpcode,
    /// `#NM`: floating-point state is unavailable; nothing raises it today.
    DeviceNotAvailable,
    /// `#DF`: raised while handling one, on its own stack since the usual one is the likely cause.
    DoubleFault,
    /// `#TS`: a task-state segment the CPU would not load.
    InvalidTaskState,
    /// `#NP`: a segment descriptor marked absent.
    SegmentNotPresent,
    /// `#SS`: a stack-segment violation.
    StackSegment,
    /// `#GP`: the catch-all protection violation, and what a non-canonical address becomes.
    GeneralProtection,
    /// `#PF`: `CR2` holds the address, and the decoded error code says why.
    PageFault(PageFault),
    /// `#MF`: a pending x87 error, raised at the next floating-point instruction.
    X87FloatingPoint,
    /// `#AC`: an unaligned access with alignment checking enabled.
    AlignmentCheck,
    /// `#MC`: the hardware reporting its own failure.
    MachineCheck,
    /// `#XM`: an SSE error.
    SimdFloatingPoint,
    /// `#VE`: an EPT violation delivered to the guest.
    Virtualisation,
    /// `#CP`: a control-flow violation, where CET shadow stacks will arrive (`010` §10).
    ControlProtection,
    /// `#HV`: hypervisor injection.
    HypervisorInjection,
    /// `#VC`: VMM communication, under SEV-ES.
    VmmCommunication,
    /// `#SX`: a security exception.
    Security,
}

impl Fault {
    /// The vector the architecture raises this on.
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

    /// What to call this in a report; a host test keeps every variant named in `x86_tables`.
    pub const fn name(self) -> &'static str {
        match exception::name(self.vector()) {
            Some(name) => name,
            None => "exception",
        }
    }
}

/// Why a page fault happened, from the error code bits of Intel SDM Vol. 3A §4.7.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageFault(u64);

impl PageFault {
    /// Whether the code is a ring-0 data access, refusing every privilege, fetch or future bit.
    pub const fn is_supervisor_data_access(self) -> bool {
        self.0 & !0b11 == 0
    }

    /// A page was present and the access violated it; clear means nothing was mapped.
    pub const fn was_present(self) -> bool {
        self.0 & 1 != 0
    }

    /// The access was a write.
    pub const fn was_write(self) -> bool {
        self.0 & (1 << 1) != 0
    }

    /// The access came from ring 3.
    pub const fn from_user(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    /// A reserved bit was set in a page-table entry on the walk.
    pub const fn reserved_bit_set(self) -> bool {
        self.0 & (1 << 3) != 0
    }

    /// The access was an instruction fetch. Meaningful only with NX enabled.
    pub const fn was_fetch(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    /// A protection key denied the access.
    pub const fn protection_key(self) -> bool {
        self.0 & (1 << 5) != 0
    }

    /// A shadow-stack access. Set when CET shadow stacks are enabled.
    pub const fn shadow_stack(self) -> bool {
        self.0 & (1 << 6) != 0
    }

    pub const fn bits(self) -> u64 {
        self.0
    }
}

/// Reads as the sentence a fault report wants.
impl fmt::Debug for PageFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
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
            f.write_str(", reserved bit set in a table entry")?;
        }
        if self.protection_key() {
            f.write_str(", protection key")?;
        }
        if self.shadow_stack() {
            f.write_str(", shadow stack")?;
        }
        Ok(())
    }
}

/// A page fault presented to a controlled recovery boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecoveryAttempt {
    pub vector: u64,
    pub frame_error_code: u64,
    pub code_selector: u64,
    pub instruction_pointer: u64,
    /// The page-fault cause decoded by the trap dispatcher.
    pub page_fault: PageFault,
    /// The address saved in `CR2`.
    pub faulting_address: u64,
}

impl RecoveryAttempt {
    /// Whether every architectural value binds this attempt to `rule`.
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

/// The page-fault class one controlled recovery site admits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum RecoveryFault {
    /// Any present/missing, read/write supervisor data access.
    SupervisorData,
    /// Exactly a present supervisor write-protection violation.
    SupervisorWriteProtection,
}

/// The complete non-liveness policy for one controlled recovery site.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecoveryRule {
    pub fault: RecoveryFault,
    /// The half-open `CR2` interval owned by the operation.
    pub fault_addresses: Range<u64>,
    /// The half-open faulting-instruction interval owned by the operation.
    pub instruction_addresses: Range<u64>,
}

#[cfg(test)]
mod tests;
