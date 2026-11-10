//! What a trap was, decoded from the vector and error code the CPU pushed.
//!
//! The entry path saves one frame shape for every vector (D-29). Central
//! decoding keeps architectural meaning out of call sites and preserves the
//! page-fault error code.
//!
//! The pure decode lives outside `wost` so host tests can reach it; handlers
//! consume one decoded cause instead of reinterpreting vector numbers (D-41,
//! D-81).
//!
//! # Examples
//!
//! ```
//! use trap_cause::{Fault, TrapCause};
//!
//! // A write to a page that was present: a protection violation, not a miss.
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

use core::ops::Range;

use x86_tables::exception;

/// Why the CPU entered the kernel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum TrapCause {
    /// An architectural exception the CPU raised.
    Fault(Fault),
    /// A vector the architecture reserves but does not define.
    ///
    /// Kept distinct from an interrupt: a reserved vector arriving is a
    /// statement about the machine, not about a device.
    Reserved(u8),
    /// A device or software interrupt, by vector.
    Interrupt(u8),
}

impl TrapCause {
    /// Decode `vector` and the error code the CPU pushed with it.
    ///
    /// `error_code` is ignored for every vector that does not push one, so a
    /// caller that always passes the saved word is correct.
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
    /// `#DE` — a division whose result does not fit, or by zero.
    DivideError,
    /// `#DB` — a debug condition: a hardware breakpoint or a single step.
    Debug,
    /// A non-maskable interrupt. Not masked by the interrupt flag, so no lock
    /// excludes it.
    NonMaskable,
    /// `#BP` — the trap `int3` plants. The one resumable entry here.
    Breakpoint,
    /// `#OF` — `into` with the overflow flag set.
    Overflow,
    /// `#BR` — a `bound` check that failed.
    BoundRange,
    /// `#UD` — an opcode the CPU does not implement in this mode.
    InvalidOpcode,
    /// `#NM` — floating-point state is unavailable. Reserved for a future
    /// XFD-based allocator (D-82); nothing raises it today.
    DeviceNotAvailable,
    /// `#DF` — an exception raised while handling one. It has a stack of its
    /// own because the usual one is overwhelmingly why it happened (D-89).
    DoubleFault,
    /// `#TS` — a task-state segment the CPU would not load.
    InvalidTaskState,
    /// `#NP` — a segment descriptor marked absent.
    SegmentNotPresent,
    /// `#SS` — a stack-segment violation.
    StackSegment,
    /// `#GP` — the catch-all protection violation, and what a non-canonical
    /// address becomes.
    GeneralProtection,
    /// `#PF` — the page tables could not satisfy an access. `CR2` holds the
    /// address; the decoded error code says why.
    PageFault(PageFault),
    /// `#MF` — a pending x87 error, raised at the next floating-point
    /// instruction rather than where it happened.
    X87FloatingPoint,
    /// `#AC` — an unaligned access with alignment checking enabled.
    AlignmentCheck,
    /// `#MC` — the hardware reporting its own failure.
    MachineCheck,
    /// `#XM` — an SSE error.
    SimdFloatingPoint,
    /// `#VE` — an EPT violation delivered to the guest.
    Virtualisation,
    /// `#CP` — a control-flow violation. What CET raises, so this is the
    /// vector shadow stacks will arrive on (`010` §10).
    ControlProtection,
    /// `#HV` — hypervisor injection.
    HypervisorInjection,
    /// `#VC` — VMM communication, under SEV-ES.
    VmmCommunication,
    /// `#SX` — a security exception.
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

    /// What to call this in a report.
    ///
    /// Falls back to a generic word rather than panicking, which cannot
    /// happen: every variant's vector is named in `x86_tables::exception`, and
    /// a host test walks all of them to keep that true.
    pub const fn name(self) -> &'static str {
        match exception::name(self.vector()) {
            Some(name) => name,
            None => "exception",
        }
    }
}

/// Why a page fault happened, from the error code the CPU pushed.
///
/// Bit meanings are Intel SDM Vol. 3A §4.7. The useful reading is usually the
/// first bit: clear means nothing was mapped there, set means something was and
/// the access was not allowed. A miss and a refusal are very different bugs,
/// and until this crate existed a fault report could not tell them apart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageFault(u64);

impl PageFault {
    /// Whether the error code describes a ring-0 data access.
    ///
    /// Present/missing and read/write may vary. Every privilege, fetch,
    /// reserved, key, shadow-stack, SGX, or future bit is refused.
    pub const fn is_supervisor_data_access(self) -> bool {
        self.0 & !0b11 == 0
    }

    /// A page was present, and the access violated its permissions.
    ///
    /// Clear means the translation was absent — a miss rather than a refusal.
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

    /// The bits themselves, for a report that wants them raw.
    pub const fn bits(self) -> u64 {
        self.0
    }
}

impl core::fmt::Debug for PageFault {
    /// Reads as the sentence a fault report wants, not as a bit dump.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
            core::fmt::Write::write_str(f, ", reserved bit set in a table entry")?;
        }
        if self.protection_key() {
            core::fmt::Write::write_str(f, ", protection key")?;
        }
        if self.shadow_stack() {
            core::fmt::Write::write_str(f, ", shadow stack")?;
        }
        Ok(())
    }
}

/// A page fault presented to a controlled recovery boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecoveryAttempt {
    /// The vector saved in the trap frame.
    pub vector: u64,
    /// The error code saved in the trap frame.
    pub frame_error_code: u64,
    /// The selector saved in the trap frame.
    pub code_selector: u64,
    /// The instruction pointer saved in the trap frame.
    pub instruction_pointer: u64,
    /// The page-fault cause decoded by the trap dispatcher.
    pub page_fault: PageFault,
    /// The address saved in `CR2`.
    pub faulting_address: u64,
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
    /// The admitted page-fault class.
    pub fault: RecoveryFault,
    /// The half-open `CR2` interval owned by the operation.
    pub fault_addresses: Range<u64>,
    /// The half-open faulting-instruction interval owned by the operation.
    pub instruction_addresses: Range<u64>,
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

#[cfg(test)]
mod tests;
