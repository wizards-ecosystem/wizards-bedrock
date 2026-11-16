use super::*;

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

#[test]
fn recovery_requires_cr2_inside_the_owned_interval() {
    const BITS: u64 = 0b10;
    let rule = RecoveryRule {
        fault_addresses: 0x2000..0x3000,
        instruction_addresses: 0x8000..0x8002,
    };
    let start = rule.fault_addresses.start;
    let end = rule.fault_addresses.end;
    for address in [start, end - 1] {
        assert!(attempt(BITS, address, rule.instruction_addresses.start).is_admitted_by(&rule));
    }
    for address in [start - 1, end, u64::MAX] {
        assert!(
            !attempt(BITS, address, rule.instruction_addresses.start).is_admitted_by(&rule),
            "CR2 {address:#x}"
        );
    }

    let mut empty = rule.clone();
    empty.fault_addresses = start..start;
    assert!(!attempt(BITS, start, rule.instruction_addresses.start).is_admitted_by(&empty));
    empty.fault_addresses = end..start;
    assert!(!attempt(BITS, start, rule.instruction_addresses.start).is_admitted_by(&empty));
}
