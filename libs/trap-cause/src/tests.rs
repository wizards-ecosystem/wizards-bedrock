use super::*;

/// Every vector decodes to something that reports its own number back.
///
/// The round trip is the point. Two hundred and fifty-six stubs each
/// knowing their own vector made this true by construction; a decode can
/// get it wrong, and a fault report naming the wrong exception is the kind
/// of error that costs an afternoon.
/// Every vector decodes to the variant the architecture assigns it.
///
/// **Transcribed from SDM Vol. 3A table 6-1**, not derived. Bidirectional
/// implementation checks admit a consistent permutation, while fault
/// containment relies on the decoded variant (D-136).
#[test]
fn every_fault_sits_on_the_vector_the_manual_gives_it() {
    // SDM Vol. 3A, table 6-1. The numbers are the architecture's, so they
    // are written out rather than referred to.
    let pairs: [(u8, Fault); 21] = [
        (0, Fault::DivideError),
        (1, Fault::Debug),
        (2, Fault::NonMaskable),
        (3, Fault::Breakpoint),
        (4, Fault::Overflow),
        (5, Fault::BoundRange),
        (6, Fault::InvalidOpcode),
        (7, Fault::DeviceNotAvailable),
        (8, Fault::DoubleFault),
        (10, Fault::InvalidTaskState),
        (11, Fault::SegmentNotPresent),
        (12, Fault::StackSegment),
        (13, Fault::GeneralProtection),
        (16, Fault::X87FloatingPoint),
        (17, Fault::AlignmentCheck),
        (18, Fault::MachineCheck),
        (19, Fault::SimdFloatingPoint),
        (20, Fault::Virtualisation),
        (21, Fault::ControlProtection),
        (28, Fault::HypervisorInjection),
        (29, Fault::VmmCommunication),
    ];

    for (vector, fault) in pairs {
        // Both directions, against the literal rather than each other.
        assert_eq!(
            fault.vector(),
            vector,
            "{fault:?} does not sit on vector {vector}"
        );
        assert_eq!(
            TrapCause::decode(vector, 0),
            TrapCause::Fault(fault),
            "vector {vector} does not decode to {fault:?}"
        );
    }

    // The two the table above cannot spell: 14 carries an error code, and
    // 30 is the last architectural vector.
    let TrapCause::Fault(page_fault) = TrapCause::decode(14, 0) else {
        panic!("vector 14 is the page fault");
    };
    assert_eq!(page_fault.vector(), 14);
    assert!(matches!(page_fault, Fault::PageFault(_)));
    assert_eq!(Fault::Security.vector(), 30);
    assert_eq!(TrapCause::decode(30, 0), TrapCause::Fault(Fault::Security));

    // The gaps stay gaps: 9, 15, 22..=27 and 31 are reserved, and a
    // variant there would mean the table was renumbered around it.
    for reserved in [9u8, 15, 22, 23, 24, 25, 26, 27, 31] {
        assert!(
            matches!(TrapCause::decode(reserved, 0), TrapCause::Reserved(_)),
            "vector {reserved} is reserved and decoded as something else"
        );
    }
}

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

#[test]
fn only_exact_supervisor_data_faults_are_admitted() {
    for bits in 0_u64..=u16::MAX as u64 {
        assert_eq!(
            PageFault(bits).is_supervisor_data_access(),
            bits <= 0b11,
            "error code {bits:#x}"
        );
    }
}

fn page_fault(bits: u64) -> PageFault {
    let TrapCause::Fault(Fault::PageFault(fault)) = TrapCause::decode(exception::PAGE_FAULT, bits)
    else {
        panic!("the page-fault vector decodes as a page fault");
    };
    fault
}

fn attempt(bits: u64, faulting_address: u64, instruction_pointer: u64) -> RecoveryAttempt {
    RecoveryAttempt {
        vector: u64::from(exception::PAGE_FAULT),
        frame_error_code: bits,
        code_selector: 0x10,
        instruction_pointer,
        page_fault: page_fault(bits),
        faulting_address,
    }
}

fn user_copy_rule() -> RecoveryRule {
    RecoveryRule {
        fault: RecoveryFault::SupervisorData,
        fault_addresses: 0x2000..0x3000,
        instruction_addresses: 0x8000..0x8002,
    }
}

fn write_protection_rule() -> RecoveryRule {
    RecoveryRule {
        fault: RecoveryFault::SupervisorWriteProtection,
        fault_addresses: 0xd00d..0xd00e,
        instruction_addresses: 0x9000..0x9001,
    }
}

#[test]
fn recovery_requires_the_page_fault_vector() {
    let cases = [
        (attempt(0b10, 0x2345, 0x8001), user_copy_rule()),
        (attempt(0b11, 0xd00d, 0x9000), write_protection_rule()),
    ];
    for (mut candidate, rule) in cases {
        assert!(candidate.is_admitted_by(&rule));
        for vector in [0, 13, 15, u64::MAX] {
            candidate.vector = vector;
            assert!(!candidate.is_admitted_by(&rule), "vector {vector}");
        }
    }
}

#[test]
fn recovery_requires_the_error_code_the_dispatcher_decoded() {
    let cases = [
        (attempt(0b10, 0x2345, 0x8001), user_copy_rule()),
        (attempt(0b11, 0xd00d, 0x9000), write_protection_rule()),
    ];
    for (mut candidate, rule) in cases {
        assert!(candidate.is_admitted_by(&rule));
        candidate.frame_error_code ^= 1;
        assert!(!candidate.is_admitted_by(&rule));
    }
}

#[test]
fn recovery_requires_a_ring_zero_frame() {
    let cases = [
        (attempt(0b10, 0x2345, 0x8001), user_copy_rule()),
        (attempt(0b11, 0xd00d, 0x9000), write_protection_rule()),
    ];
    for (mut candidate, rule) in cases {
        assert!(candidate.is_admitted_by(&rule));
        for selector in [1, 2, 3, 0x11, 0x12, 0x13, u64::MAX] {
            candidate.code_selector = selector;
            assert!(!candidate.is_admitted_by(&rule), "selector {selector:#x}");
        }
    }
}

#[test]
fn user_copy_recovery_accepts_only_supervisor_data_faults() {
    let rule = user_copy_rule();
    for bits in (0_u64..=u16::MAX as u64).chain((16..64).map(|bit| 1_u64 << bit)) {
        assert_eq!(
            attempt(bits, 0x2345, 0x8001).is_admitted_by(&rule),
            bits & !0b11 == 0,
            "error code {bits:#x}"
        );
    }
}

#[test]
fn write_protection_recovery_requires_the_exact_protection_write() {
    let rule = write_protection_rule();
    for bits in (0_u64..=u16::MAX as u64).chain((16..64).map(|bit| 1_u64 << bit)) {
        assert_eq!(
            attempt(bits, 0xd00d, 0x9000).is_admitted_by(&rule),
            bits == 0b11,
            "error code {bits:#x}"
        );
    }
}

#[test]
fn recovery_requires_cr2_inside_the_owned_interval() {
    let cases = [(0b10, user_copy_rule()), (0b11, write_protection_rule())];
    for (bits, rule) in cases {
        let start = rule.fault_addresses.start;
        let end = rule.fault_addresses.end;
        for address in [start, end - 1] {
            assert!(attempt(bits, address, rule.instruction_addresses.start).is_admitted_by(&rule));
        }
        for address in [start - 1, end, u64::MAX] {
            assert!(
                !attempt(bits, address, rule.instruction_addresses.start).is_admitted_by(&rule),
                "CR2 {address:#x}"
            );
        }

        let mut empty = rule.clone();
        empty.fault_addresses = start..start;
        assert!(!attempt(bits, start, rule.instruction_addresses.start).is_admitted_by(&empty));
        empty.fault_addresses = end..start;
        assert!(!attempt(bits, start, rule.instruction_addresses.start).is_admitted_by(&empty));
    }
}

#[test]
fn recovery_requires_rip_inside_the_owned_interval() {
    let cases = [
        (0b10, 0x2345, user_copy_rule()),
        (0b11, 0xd00d, write_protection_rule()),
    ];
    for (bits, address, rule) in cases {
        let start = rule.instruction_addresses.start;
        let end = rule.instruction_addresses.end;
        for instruction in [start, end - 1] {
            assert!(attempt(bits, address, instruction).is_admitted_by(&rule));
        }
        for instruction in [start - 1, end, u64::MAX] {
            assert!(
                !attempt(bits, address, instruction).is_admitted_by(&rule),
                "RIP {instruction:#x}"
            );
        }

        let mut empty = rule.clone();
        empty.instruction_addresses = start..start;
        assert!(!attempt(bits, address, start).is_admitted_by(&empty));
        empty.instruction_addresses = end..start;
        assert!(!attempt(bits, address, start).is_admitted_by(&empty));
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
///
/// `Fault::name` is `exception::name(self.vector()).unwrap_or("exception")`
/// and the `if let` guard fires only where that is `Some`, so the equality
/// alone cannot fail. The transcribed names below are what makes it a
/// check: two crates naming the same table is why `trap-cause` exists
/// (D-105).
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

    // SDM Vol. 3A table 6-1, spelled out here rather than derived.
    let named = |vector: u8| match TrapCause::decode(vector, 0) {
        TrapCause::Fault(fault) => Some(fault.name()),
        _ => None,
    };
    assert_eq!(named(0), Some("divide error"));
    assert_eq!(named(6), Some("invalid opcode"));
    assert_eq!(named(8), Some("double fault"));
    assert_eq!(named(13), Some("general protection fault"));
    assert_eq!(named(14), Some("page fault"));
}

#[test]
fn every_decoded_cause_has_the_architectural_report_name() {
    for vector in 0..=u8::MAX {
        let expected = if vector >= exception::COUNT {
            "interrupt"
        } else {
            exception::name(vector).unwrap_or("reserved vector")
        };
        assert_eq!(
            TrapCause::decode(vector, 0).name(),
            expected,
            "vector {vector}"
        );
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
