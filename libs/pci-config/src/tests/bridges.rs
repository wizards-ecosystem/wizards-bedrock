use super::*;

#[derive(Debug)]
struct SoleFunction {
    at: Address,
    identity: u32,
    class: u32,
    command: u16,
    status: Option<u32>,
    quiesces: u32,
    written: Option<u16>,
}

impl SoleFunction {
    fn new(at: Address, identity: u32, class: u32, command: u16) -> Self {
        Self {
            at,
            identity,
            class,
            command,
            status: None,
            quiesces: 0,
            written: None,
        }
    }

    fn quiescing(self, status: Option<u32>) -> Self {
        Self { status, ..self }
    }

    fn access(&mut self, candidate: Address, operation: HandoffAccess) -> Option<u32> {
        if candidate.function_base() != self.at {
            return elsewhere(operation);
        }
        match operation {
            HandoffAccess::ReadIdentity => Some(self.identity),
            HandoffAccess::ReadClass => Some(self.class),
            HandoffAccess::ReadCommand => Some(u32::from(self.command)),
            HandoffAccess::WriteCommand(value) => {
                self.written = Some(value);
                Some(0)
            }
            HandoffAccess::QuiesceIsaDma => {
                self.quiesces += 1;
                self.status
            }
        }
    }

    fn fence(
        &mut self,
        policy: HandoffFencePolicy,
    ) -> Result<HandoffFenceReport, HandoffFenceError> {
        fence_dma_handoff(policy, |at, operation| self.access(at, operation))
    }
}

fn elsewhere(operation: HandoffAccess) -> Option<u32> {
    matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX)
}

fn observation(at: Address, identity: u32, class: u32, command: u16) -> FunctionObservation {
    FunctionObservation::new(
        at,
        identity,
        class,
        command,
        Class::from_bits(class),
        HeaderType::from_bits(0),
        None,
    )
}

#[test]
fn only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester() {
    let isa = Address::new(0, 7, 0, 0).expect("VMware ISA bridge");
    let identity = 0x7110_8086;
    let class = 0x06_01_00_00;
    let command = command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER;
    let quiet = |at, identity, class, command| {
        SoleFunction::new(at, identity, class, command).quiescing(Some(0))
    };

    let mut bridge = SoleFunction::new(isa, identity, class, command).quiescing(Some(0x0f0f));
    let mut report = bridge
        .fence(HandoffFencePolicy::vmware_440bx())
        .expect("the exact ISA bridge is quiesced at its 8237 controllers");
    assert_eq!(report.functions(), 1);
    assert_eq!(report.disabled(), 0);
    assert_eq!(report.retained_non_requesters(), 0);
    assert_eq!(report.quiesced_requesters(), 1);
    assert_eq!(bridge.quiesces, 1);
    assert_eq!(
        bridge.written,
        Some(command::IO_SPACE | command::MEMORY_SPACE)
    );

    let authority = report
        .take_quiesced_requester()
        .expect("entry fence retained its exact quiesce authority");
    assert_eq!(authority.address(), isa);
    let mut topology = PciTopology::new();
    topology
        .observe_quiesced_requester(authority, observation(isa, identity, class, command))
        .expect("quiesced requester topology observation");
    assert_eq!(topology.quiesced_requester(isa), None);
    topology.seal().expect("root-bus topology");
    assert!(topology.contains_function(isa));
    assert!(topology.contains_requester(isa));
    let record = topology
        .quiesced_requester(isa)
        .expect("the sealed snapshot names the quiesced requester");
    assert_eq!(
        (record.address(), record.identity(), record.command()),
        (isa, identity, command)
    );
    let other = Address::new(0, 7, 1, 0).expect("IDE function");
    assert_eq!(topology.quiesced_requester(other), None);

    for (at, later_identity, later_class, later_command) in [
        (other, identity, class, command),
        (isa, identity ^ 1, class, command),
        (isa, identity, class ^ 1, command),
        (isa, identity, class, command & !command::IO_SPACE),
    ] {
        let changed = quiet(isa, identity, class, command)
            .fence(HandoffFencePolicy::vmware_440bx())
            .expect("exact ISA bridge")
            .take_quiesced_requester()
            .expect("exact quiesce authority");
        assert_eq!(
            PciTopology::new().observe_quiesced_requester(
                changed,
                observation(at, later_identity, later_class, later_command),
            ),
            Err(TopologyError::QuiescedRequesterChanged {
                at: at.function_base(),
            })
        );
    }

    for revision in [0u32, 1, u32::from(u8::MAX)] {
        let report = quiet(isa, identity, class | revision, command)
            .fence(HandoffFencePolicy::vmware_440bx())
            .expect("every PIIX4 revision is the same ISA bridge");
        assert_eq!(report.quiesced_requesters(), 1);
    }

    for (at, identity, class, command) in [
        (isa, identity ^ 1, class, command),
        (isa, identity ^ (1 << 16), class, command),
        (isa, identity, class ^ (1 << 24), command),
        (isa, identity, class ^ (1 << 16), command),
        (isa, identity, class ^ (1 << 8), command),
        (isa, identity, class, command & !command::IO_SPACE),
        (isa, identity, class, command & !command::MEMORY_SPACE),
        (
            Address::new(1, 7, 0, 0).expect("nearby bus"),
            identity,
            class,
            command,
        ),
        (
            Address::new(0, 6, 0, 0).expect("nearby device"),
            identity,
            class,
            command,
        ),
        (other, identity, class, command),
    ] {
        let mut near_miss = quiet(at, identity, class, command);
        let refused = near_miss.fence(HandoffFencePolicy::vmware_440bx());
        assert!(matches!(
            refused,
            Err(HandoffFenceError::CommandRefused { .. })
        ));
        assert_eq!(
            near_miss.quiesces, 0,
            "a near miss never touches the 8237 controllers"
        );
    }

    let mut strict_bridge = quiet(isa, identity, class, command);
    let strict = strict_bridge.fence(HandoffFencePolicy::strict());
    assert!(matches!(
        strict,
        Err(HandoffFenceError::CommandRefused { .. })
    ));
    assert_eq!(strict_bridge.quiesces, 0);
    assert!(!HandoffFencePolicy::strict().quiesces_isa_dma());

    for (first, second) in [
        (command & !command::IO_SPACE, command),
        (command, command & !command::IO_SPACE),
    ] {
        let mut reads = 0;
        let mut quiesces = 0;
        let refused = fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
            if at.function_base() != isa {
                return elsewhere(operation);
            }
            match operation {
                HandoffAccess::ReadIdentity => Some(identity),
                HandoffAccess::ReadClass => Some(class),
                HandoffAccess::ReadCommand => {
                    reads += 1;
                    Some(u32::from(if reads == 1 { first } else { second }))
                }
                HandoffAccess::WriteCommand(_) => Some(0),
                HandoffAccess::QuiesceIsaDma => {
                    quiesces += 1;
                    Some(0)
                }
            }
        });
        assert!(matches!(
            refused,
            Err(HandoffFenceError::CommandRefused { .. })
        ));
        assert_eq!(quiesces, 0);
    }

    for pending in [0x0010, 0x0080, 0x1000, 0x8000, 0xf0f0] {
        let refused = SoleFunction::new(isa, identity, class, command)
            .quiescing(Some(pending))
            .fence(HandoffFencePolicy::vmware_440bx());
        assert_eq!(
            refused.err(),
            Some(HandoffFenceError::IsaDmaPending {
                at: isa,
                status: pending,
            })
        );
    }

    let unavailable =
        SoleFunction::new(isa, identity, class, command).fence(HandoffFencePolicy::vmware_440bx());
    assert_eq!(
        unavailable.err(),
        Some(HandoffFenceError::Access {
            at: isa,
            operation: HandoffAccess::QuiesceIsaDma,
        })
    );
    assert_eq!(
        HandoffFenceError::IsaDmaPending {
            at: isa,
            status: 0x0010,
        }
        .to_string(),
        "00:07.0 8237 status 0x0010 still requests DMA after masking"
    );
}

#[test]
fn only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command() {
    let host = Address::new(0, 0, 0, 0).expect("VMware host bridge");
    let identity = 0x7190_8086;
    let class = 0x06_00_00_00;
    let command = command::MEMORY_SPACE | command::BUS_MASTER;

    let mut bridge = SoleFunction::new(host, identity, class, command);
    let mut report = bridge
        .fence(HandoffFencePolicy::vmware_440bx())
        .expect("the qualified host bridge is not a PCI DMA requester");

    assert_eq!(report.functions(), 1);
    assert_eq!(report.disabled(), 0);
    assert_eq!(report.retained_non_requesters(), 1);
    assert_eq!(bridge.written, Some(command::MEMORY_SPACE));

    for revision in [0u32, 1, u32::from(u8::MAX)] {
        assert!(
            HandoffFencePolicy::vmware_440bx()
                .classify_non_requester(host, identity, class | revision, command, command,)
                .is_some()
        );
    }

    let authority = report
        .take_non_requester()
        .expect("entry fence retained its exact classification authority");
    assert_eq!(report.retained_non_requesters(), 1);
    let mut topology = PciTopology::new();
    topology
        .observe_non_requester(authority, observation(host, identity, class, command))
        .expect("non-requester topology observation");
    topology.seal().expect("root-bus topology");
    assert!(topology.contains_function(host));
    assert!(!topology.contains_requester(host));

    for (at, later_identity, later_class, later_command) in [
        (
            Address::new(0, 1, 0, 0).expect("different BDF"),
            identity,
            class,
            command,
        ),
        (host, identity ^ 1, class, command),
        (host, identity, class ^ 1, command),
        (host, identity, class, command | command::IO_SPACE),
    ] {
        let changed_authority = HandoffFencePolicy::vmware_440bx()
            .classify_non_requester(host, identity, class, command, command)
            .expect("exact host bridge classification");
        let mut changed_topology = PciTopology::new();
        assert_eq!(
            changed_topology.observe_non_requester(
                changed_authority,
                observation(at, later_identity, later_class, later_command),
            ),
            Err(TopologyError::QualifiedNonRequesterChanged {
                at: at.function_base(),
            })
        );
    }

    for (at, identity, class, command) in [
        (host, identity ^ 1, class, command),
        (host, identity ^ (1 << 16), class, command),
        (host, identity, class ^ (1 << 24), command),
        (host, identity, class ^ (1 << 16), command),
        (host, identity, class ^ (1 << 8), command),
        (host, identity, class, command | command::IO_SPACE),
        (
            Address::new(1, 0, 0, 0).expect("nearby bus"),
            identity,
            class,
            command,
        ),
        (
            Address::new(0, 1, 0, 0).expect("nearby function"),
            identity,
            class,
            command,
        ),
        (
            Address::new(0, 0, 1, 0).expect("nearby function"),
            identity,
            class,
            command,
        ),
    ] {
        let refused = SoleFunction::new(at, identity, class, command)
            .fence(HandoffFencePolicy::vmware_440bx());
        assert!(matches!(
            refused,
            Err(HandoffFenceError::CommandRefused { .. })
        ));
    }

    let strict =
        SoleFunction::new(host, identity, class, command).fence(HandoffFencePolicy::strict());
    assert!(matches!(
        strict,
        Err(HandoffFenceError::CommandRefused { .. })
    ));

    let mut unexpected_before_reads = 0;
    let unexpected_before =
        fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
            if at.function_base() != host {
                return elsewhere(operation);
            }
            match operation {
                HandoffAccess::ReadIdentity => Some(identity),
                HandoffAccess::ReadClass => Some(class),
                HandoffAccess::ReadCommand => {
                    unexpected_before_reads += 1;
                    Some(u32::from(if unexpected_before_reads == 1 {
                        command | command::IO_SPACE
                    } else {
                        command
                    }))
                }
                HandoffAccess::WriteCommand(_) => Some(0),
                HandoffAccess::QuiesceIsaDma => None,
            }
        });
    assert!(matches!(
        unexpected_before,
        Err(HandoffFenceError::CommandRefused { .. })
    ));

    let mut command_reads = 0;
    let changed_command = fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
        if at.function_base() != host {
            return elsewhere(operation);
        }
        match operation {
            HandoffAccess::ReadIdentity => Some(identity),
            HandoffAccess::ReadClass => Some(class),
            HandoffAccess::ReadCommand => {
                command_reads += 1;
                Some(u32::from(if command_reads == 1 {
                    command
                } else {
                    command | command::IO_SPACE
                }))
            }
            HandoffAccess::WriteCommand(_) => Some(0),
            HandoffAccess::QuiesceIsaDma => None,
        }
    });
    assert!(matches!(
        changed_command,
        Err(HandoffFenceError::CommandRefused { .. })
    ));

    let mut class_reads = 0;
    let changed_class = fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
        if at.function_base() != host {
            return elsewhere(operation);
        }
        match operation {
            HandoffAccess::ReadIdentity => Some(identity),
            HandoffAccess::ReadClass => {
                class_reads += 1;
                Some(if class_reads == 1 { class } else { class ^ 1 })
            }
            HandoffAccess::ReadCommand => Some(u32::from(command)),
            HandoffAccess::WriteCommand(_) => Some(0),
            HandoffAccess::QuiesceIsaDma => None,
        }
    });
    assert!(matches!(
        changed_class,
        Err(HandoffFenceError::ClassChanged {
            at,
            expected,
            found,
        }) if at == host && expected == class && found == class ^ 1
    ));
}
