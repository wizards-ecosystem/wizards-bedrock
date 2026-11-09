use super::*;

#[test]
fn the_handoff_fence_exhausts_the_segment_and_preserves_command_bits() {
    let first = Address::new(0, 1, 0, 0).expect("first function");
    let last = Address::new(u8::MAX, DEVICES - 1, FUNCTIONS - 1, 0).expect("last function");
    let first_identity = 0x1234_5678;
    let last_identity = 0x89ab_cdef;
    let mut first_command =
        command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER | (1 << 10);
    let mut last_command = command::MEMORY_SPACE | (1 << 8);

    let report = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
        let function = at.function_base();
        let identity = if function == first {
            first_identity
        } else if function == last {
            last_identity
        } else {
            u32::MAX
        };
        match operation {
            HandoffAccess::ReadIdentity => Some(identity),
            HandoffAccess::ReadClass => Some(0),
            HandoffAccess::ReadCommand if function == first => Some(u32::from(first_command)),
            HandoffAccess::ReadCommand if function == last => Some(u32::from(last_command)),
            HandoffAccess::WriteCommand(value) if function == first => {
                first_command = value;
                Some(0)
            }
            HandoffAccess::WriteCommand(value) if function == last => {
                last_command = value;
                Some(0)
            }
            _ => None,
        }
    })
    .expect("the complete static segment is fenced");

    assert_eq!(report.functions(), 2);
    assert_eq!(report.disabled(), 1);
    assert_eq!(report.retained_non_requesters(), 0);
    assert_eq!(
        first_command,
        command::IO_SPACE | command::MEMORY_SPACE | (1 << 10)
    );
    assert_eq!(last_command, command::MEMORY_SPACE | (1 << 8));
}

#[test]
fn every_handoff_fence_uncertainty_refuses_the_boundary() {
    let target = Address::new(0, 0, 0, 0).expect("target function");
    let identity = 0x1234_5678;

    let access = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
        if at.function_base() != target {
            return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
        }
        match operation {
            HandoffAccess::ReadIdentity => Some(identity),
            HandoffAccess::ReadClass => Some(0),
            HandoffAccess::ReadCommand => None,
            HandoffAccess::WriteCommand(_) => Some(0),
            HandoffAccess::QuiesceIsaDma => None,
        }
    });
    assert!(matches!(
        access,
        Err(HandoffFenceError::Access {
            operation: HandoffAccess::ReadCommand,
            ..
        })
    ));

    let refused = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
        if at.function_base() != target {
            return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
        }
        match operation {
            HandoffAccess::ReadIdentity => Some(identity),
            HandoffAccess::ReadClass => Some(0),
            HandoffAccess::ReadCommand => Some(u32::from(command::BUS_MASTER)),
            HandoffAccess::WriteCommand(_) => Some(0),
            HandoffAccess::QuiesceIsaDma => None,
        }
    });
    assert!(matches!(
        refused,
        Err(HandoffFenceError::CommandRefused {
            at,
            identity: found_identity,
            class: 0,
            expected: 0,
            found: command::BUS_MASTER,
        }) if at == target && found_identity == identity
    ));

    let mut identity_reads = 0;
    let changed = fence_dma_handoff(HandoffFencePolicy::strict(), |at, operation| {
        if at.function_base() != target {
            return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
        }
        match operation {
            HandoffAccess::ReadIdentity => {
                identity_reads += 1;
                Some(if identity_reads == 1 {
                    identity
                } else {
                    identity ^ 1
                })
            }
            HandoffAccess::ReadClass => Some(0),
            HandoffAccess::ReadCommand => Some(0),
            HandoffAccess::WriteCommand(_) => Some(0),
            HandoffAccess::QuiesceIsaDma => None,
        }
    });
    assert!(matches!(
        changed,
        Err(HandoffFenceError::IdentityChanged {
            at,
            expected,
            found,
        }) if at == target && expected == identity && found == identity ^ 1
    ));
}

/// VMware Workstation 17.6's default segment as its entry fence saw it on
/// 2026-10-20: BDF, identity, class dword, command before, command after
/// a BME-clear write. `00:00.0` and `00:07.0` hard-wire bit 2.
type CapturedFunction = ((u8, u8, u8), u32, u32, u16, u16);

const VMWARE_SEGMENT: &[CapturedFunction] = &[
    ((0x00, 0x00, 0), 0x7190_8086, 0x0600_0001, 0x0006, 0x0006),
    ((0x00, 0x01, 0), 0x7191_8086, 0x0604_0001, 0x0017, 0x0013),
    ((0x00, 0x07, 0), 0x7110_8086, 0x0601_0008, 0x0007, 0x0007),
    ((0x00, 0x07, 1), 0x7111_8086, 0x0101_8a01, 0x0005, 0x0001),
    ((0x00, 0x07, 3), 0x7113_8086, 0x0680_0008, 0x0000, 0x0000),
    ((0x00, 0x0f, 0), 0x0405_15ad, 0x0300_0000, 0x0007, 0x0003),
    ((0x00, 0x11, 0), 0x0790_15ad, 0x0604_0102, 0x0007, 0x0003),
    ((0x02, 0x00, 0), 0x0774_15ad, 0x0c03_0000, 0x0005, 0x0001),
    ((0x02, 0x01, 0), 0x07e0_15ad, 0x0106_0100, 0x0006, 0x0002),
    ((0x03, 0x00, 0), 0x07f0_15ad, 0x0108_0200, 0x0007, 0x0003),
    ((0x0b, 0x00, 0), 0x07f0_15ad, 0x0108_0200, 0x0007, 0x0003),
];

/// The thirty-two VMware PCIe root ports at `00:15.0` through `00:18.7`.
const VMWARE_ROOT_PORT: (u32, u32, u16, u16) = (0x07a0_15ad, 0x0604_0001, 0x0007, 0x0003);

fn vmware_function(at: Address) -> Option<(u32, u32, u16, bool)> {
    let (bus, device, function) = (at.bus(), at.device(), at.function());
    if bus == 0 && (0x15..=0x18).contains(&device) {
        let (identity, class, before, _) = VMWARE_ROOT_PORT;
        return Some((identity, class, before, false));
    }
    VMWARE_SEGMENT
        .iter()
        .find(|(bdf, ..)| *bdf == (bus, device, function))
        .map(|&(_, identity, class, before, found)| {
            (
                identity,
                class,
                before,
                found == before && before & command::BUS_MASTER != 0,
            )
        })
}

#[test]
fn the_captured_vmware_segment_fences_and_fits_the_function_authority_limit() {
    let mut commands = [[[None::<u16>; FUNCTIONS as usize]; DEVICES as usize]; 256];
    let mut quiesces = 0;
    let mut report = fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
        let Some((identity, class, before, read_only)) = vmware_function(at) else {
            return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
        };
        let slot = &mut commands[usize::from(at.bus())][usize::from(at.device())]
            [usize::from(at.function())];
        match operation {
            HandoffAccess::ReadIdentity => Some(identity),
            HandoffAccess::ReadClass => Some(class),
            HandoffAccess::ReadCommand => Some(u32::from(slot.unwrap_or(before))),
            HandoffAccess::WriteCommand(value) => {
                if !read_only {
                    *slot = Some(value);
                }
                Some(0)
            }
            HandoffAccess::QuiesceIsaDma => {
                quiesces += 1;
                Some(0)
            }
        }
    })
    .expect("the captured VMware segment admits its exact policy");

    assert_eq!(report.functions(), 43);
    assert_eq!(report.disabled(), 40);
    assert_eq!(report.retained_non_requesters(), 1);
    assert_eq!(report.quiesced_requesters(), 1);
    assert_eq!(quiesces, 1);
    assert!(report.take_non_requester().is_some());
    assert!(report.take_quiesced_requester().is_some());
    let requesters = report.functions() - report.retained_non_requesters();
    assert_eq!(requesters, 42);
    assert!(
        requesters as usize <= FUNCTION_AUTHORITY_LIMIT,
        "VMware's unit governs {requesters} requesters but only {FUNCTION_AUTHORITY_LIMIT} function authorities exist"
    );
}
