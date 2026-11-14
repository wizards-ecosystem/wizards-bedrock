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
