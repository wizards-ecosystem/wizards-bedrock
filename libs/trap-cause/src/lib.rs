//! What a trap was, decoded from the vector and error code the CPU pushed.
//!
//! The entry path saves one frame shape for every vector (D-29) and hands it
//! to one handler. That handler used to match on the raw vector number, which
//! works but spreads the architecture's meaning across the call sites that
//! test for it — and it threw the page-fault error code away entirely, so a
//! fault report could say *where* but never *why*.
//!
//! This crate is the decode, and it is here rather than in `wost` for the
//! usual reason (D-41): it touches no hardware and holds no pointer, so
//! `cargo test` can reach it. It is also the shape a second architecture would
//! need — a decoded cause dispatched from a table, rather than a vector number
//! every handler re-interprets — which is why `010` §10 asks for it while the
//! entry path is being rebuilt FRED-first (D-81).
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

use x86_tables::exception;

/// Why the CPU entered the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Whether the interrupted code can be resumed.
    ///
    /// Only a breakpoint: the saved instruction pointer is already past the
    /// `int3`, so returning continues the program (D-31). Every other fault is
    /// terminal, and an interrupt is not a fault at all.
    pub const fn is_resumable(self) -> bool {
        matches!(self, Self::Fault(Fault::Breakpoint))
    }
}

/// An architectural exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// The access was an instruction fetch. Meaningful only with NX enabled,
    /// which it has been since Phase 2.
    pub const fn was_fetch(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    /// A protection key denied the access.
    pub const fn protection_key(self) -> bool {
        self.0 & (1 << 5) != 0
    }

    /// A shadow-stack access. Set once CET is enabled; never today.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every vector decodes to something that reports its own number back.
    ///
    /// The round trip is the point. Two hundred and fifty-six stubs each
    /// knowing their own vector made this true by construction; a decode can
    /// get it wrong, and a fault report naming the wrong exception is the kind
    /// of error that costs an afternoon.
    #[test]
    fn every_vector_decodes_and_reports_its_own_number() {
        for vector in 0..=u8::MAX {
            assert_eq!(
                TrapCause::decode(vector, 0).vector(),
                vector,
                "vector {vector} decoded to a cause reporting a different one"
            );
        }
    }

    /// Faults live in the architectural range and interrupts above it.
    #[test]
    fn the_architectural_range_is_where_faults_live() {
        for vector in 0..exception::COUNT {
            match TrapCause::decode(vector, 0) {
                TrapCause::Fault(_) | TrapCause::Reserved(_) => {}
                TrapCause::Interrupt(_) => panic!("vector {vector} decoded as an interrupt"),
            }
        }
        for vector in exception::COUNT..=u8::MAX {
            assert_eq!(TrapCause::decode(vector, 0), TrapCause::Interrupt(vector));
        }
    }

    /// A vector this crate calls reserved is one `x86_tables` has no name for.
    ///
    /// Two tables of the architecture's vectors exist now, and this is what
    /// stops them drifting apart.
    #[test]
    fn reserved_vectors_are_the_ones_with_no_name() {
        for vector in 0..exception::COUNT {
            assert_eq!(
                matches!(TrapCause::decode(vector, 0), TrapCause::Reserved(_)),
                exception::name(vector).is_none(),
                "vector {vector} disagrees with x86-tables about being reserved"
            );
        }
    }

    /// Only a breakpoint is resumable (D-31).
    #[test]
    fn only_a_breakpoint_resumes() {
        for vector in 0..=u8::MAX {
            assert_eq!(
                TrapCause::decode(vector, 0).is_resumable(),
                vector == exception::BREAKPOINT,
                "vector {vector} disagrees about being resumable"
            );
        }
    }

    /// Each page-fault bit is read on its own, and not by accident from
    /// another.
    #[test]
    fn every_page_fault_bit_is_read_on_its_own() {
        /// One bit and the accessor that is supposed to read exactly it.
        type Reader = (u64, fn(PageFault) -> bool);

        let readers: [Reader; 7] = [
            (1 << 0, PageFault::was_present),
            (1 << 1, PageFault::was_write),
            (1 << 2, PageFault::from_user),
            (1 << 3, PageFault::reserved_bit_set),
            (1 << 4, PageFault::was_fetch),
            (1 << 5, PageFault::protection_key),
            (1 << 6, PageFault::shadow_stack),
        ];
        for (bit, read) in readers {
            assert!(read(PageFault(bit)), "bit {bit:#x} did not read as set");
            assert!(
                !read(PageFault(!bit)),
                "bit {bit:#x} read as set from the others"
            );
        }
    }

    /// The error code reaches the page fault and nothing else.
    #[test]
    fn only_a_page_fault_carries_an_error_code() {
        let TrapCause::Fault(Fault::PageFault(fault)) = TrapCause::decode(14, 0x1F) else {
            panic!("vector 14 is a page fault");
        };
        assert_eq!(fault.bits(), 0x1F);

        for vector in 0..=u8::MAX {
            if vector == 14 {
                continue;
            }
            assert_eq!(
                TrapCause::decode(vector, 0),
                TrapCause::decode(vector, u64::MAX),
                "vector {vector} changed meaning with the error code"
            );
        }
    }

    /// Every fault reports the name `x86_tables` gives its vector.
    #[test]
    fn a_faults_name_is_its_vectors_name() {
        for vector in 0..exception::COUNT {
            if let TrapCause::Fault(fault) = TrapCause::decode(vector, 0) {
                assert_eq!(
                    Some(fault.name()),
                    exception::name(vector),
                    "vector {vector} reports a name that is not its own"
                );
            }
        }
    }

    /// The report reads as a sentence, and says miss and refusal differently.
    #[test]
    fn a_page_fault_reads_as_a_sentence() {
        let miss = format!("{:?}", PageFault(0));
        assert_eq!(miss, "no mapping on read from kernel");

        let refusal = format!("{:?}", PageFault(0b111));
        assert_eq!(refusal, "protection violation on write from user");

        let fetch = format!("{:?}", PageFault(0b1_0000));
        assert_eq!(fetch, "no mapping on instruction fetch from kernel");
    }
}
