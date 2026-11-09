use super::*;

fn topology_address(bus: u8, device: u8, function: u8) -> Address {
    Address::new(bus, device, function, 0).expect("fixture address")
}

fn bridge_class() -> Class {
    Class::from_bits(0x06_04_00_00)
}

fn endpoint_class() -> Class {
    Class::from_bits(0x01_08_02_00)
}

fn bridge_header() -> HeaderType {
    HeaderType::from_bits(1)
}

fn endpoint_header() -> HeaderType {
    HeaderType::from_bits(0)
}

fn bus_numbers(primary: u8, secondary: u8, subordinate: u8) -> u32 {
    u32::from(primary) | (u32::from(secondary) << 8) | (u32::from(subordinate) << 16)
}

fn assert_topology_poisoned(topology: &mut PciTopology, at: Address, error: TopologyError) {
    assert!(
        !topology.is_sealed(),
        "an error must invalidate a prior seal"
    );
    assert!(!topology.contains_function(at));
    assert!(!topology.contains_requester(at));
    assert_eq!(topology.bridge_range(at), None);
    assert!(!topology.hierarchy_contains(at, at));
    assert_eq!(topology.quiesced_requester(at), None);
    assert_eq!(topology.seal(), Err(error));
    assert!(!topology.is_sealed());
    assert_eq!(topology.seal(), Err(error), "sealing cannot clear poison");
}

#[test]
fn every_ordinary_observation_error_poisoned_until_reset() {
    let root = topology_address(0, 0x1c, 0);
    let rejected = topology_address(0, 0x1d, 0);
    for (class, header, numbers, error) in [
        (
            bridge_class(),
            endpoint_header(),
            None,
            TopologyError::BridgeIdentity { at: rejected },
        ),
        (
            bridge_class(),
            bridge_header(),
            None,
            TopologyError::MissingBusNumbers { at: rejected },
        ),
        (
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(1, 0x40, 0x40)),
            TopologyError::InvalidRange {
                at: rejected,
                primary: 1,
                secondary: 0x40,
                subordinate: 0x40,
            },
        ),
        (
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0, 0x20, 0x3f)),
            TopologyError::DuplicateParent {
                bus: 0x20,
                first: root,
                second: rejected,
            },
        ),
    ] {
        let mut topology = PciTopology::new();
        topology
            .observe(
                root,
                bridge_class(),
                bridge_header(),
                Some(bus_numbers(0, 0x20, 0x3f)),
            )
            .expect("initial bridge");
        topology.seal().expect("initial complete snapshot");
        assert!(topology.contains_requester(root));
        assert!(topology.bridge_range(root).is_some());
        assert_eq!(
            topology.observe(rejected, class, header, numbers),
            Err(error)
        );
        assert_topology_poisoned(&mut topology, root, error);
        assert!(!topology.contains_function(rejected));
        assert_eq!(
            topology.observe(rejected, endpoint_class(), endpoint_header(), None),
            Err(error),
            "a valid later observation cannot discard the first error"
        );
        assert_eq!(
            topology.observe(rejected, bridge_class(), endpoint_header(), None),
            Err(error),
            "a later different error cannot replace the first error"
        );
        topology.reset();
        topology
            .observe(rejected, endpoint_class(), endpoint_header(), None)
            .expect("reset begins an independent snapshot");
        topology.seal().expect("fresh root endpoint");
        assert!(topology.contains_requester(rejected));
        assert!(!topology.contains_function(root));
    }
}

fn topology_entry_fence() -> HandoffFenceReport {
    let host = topology_address(0, 0, 0);
    let isa = topology_address(0, 7, 0);
    fence_dma_handoff(HandoffFencePolicy::vmware_440bx(), |at, operation| {
        let (identity, class, command) = if at.function_base() == host {
            (
                0x7190_8086,
                0x06_00_00_00,
                command::MEMORY_SPACE | command::BUS_MASTER,
            )
        } else if at.function_base() == isa {
            (
                0x7110_8086,
                0x06_01_00_00,
                command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER,
            )
        } else {
            return matches!(operation, HandoffAccess::ReadIdentity).then_some(u32::MAX);
        };
        match operation {
            HandoffAccess::ReadIdentity => Some(identity),
            HandoffAccess::ReadClass => Some(class),
            HandoffAccess::ReadCommand => Some(u32::from(command)),
            HandoffAccess::WriteCommand(_) | HandoffAccess::QuiesceIsaDma => Some(0),
        }
    })
    .expect("real entry-fence token construction for the two exact platform functions")
}

fn topology_host_observation(identity: u32) -> FunctionObservation {
    FunctionObservation::new(
        topology_address(0, 0, 0),
        identity,
        0x06_00_00_00,
        command::MEMORY_SPACE | command::BUS_MASTER,
        Class::from_bits(0x06_00_00_00),
        endpoint_header(),
        None,
    )
}

fn topology_isa_observation(identity: u32) -> FunctionObservation {
    FunctionObservation::new(
        topology_address(0, 7, 0),
        identity,
        0x06_01_00_00,
        command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER,
        Class::from_bits(0x06_01_00_00),
        endpoint_header(),
        None,
    )
}

#[test]
fn a_failed_non_requester_observation_revokes_the_seal_and_retains_its_error() {
    let host = topology_address(0, 0, 0);
    let mut topology = PciTopology::new();
    topology
        .observe_non_requester(
            topology_entry_fence().take_non_requester().unwrap(),
            topology_host_observation(0x7190_8086),
        )
        .unwrap();
    topology.seal().unwrap();
    assert!(topology.contains_function(host));
    assert!(!topology.contains_requester(host));
    let error = TopologyError::QualifiedNonRequesterChanged { at: host };
    assert_eq!(
        topology.observe_non_requester(
            topology_entry_fence().take_non_requester().unwrap(),
            topology_host_observation(0x7190_8087),
        ),
        Err(error)
    );
    assert_topology_poisoned(&mut topology, host, error);
    assert_eq!(
        topology.observe_non_requester(
            topology_entry_fence().take_non_requester().unwrap(),
            topology_host_observation(0x7190_8086),
        ),
        Err(error)
    );
    assert_eq!(
        topology.observe_quiesced_requester(
            topology_entry_fence().take_quiesced_requester().unwrap(),
            topology_isa_observation(0x7110_8087),
        ),
        Err(error),
        "another observation door must preserve the first error"
    );
    topology.reset();
    topology
        .observe_non_requester(
            topology_entry_fence().take_non_requester().unwrap(),
            topology_host_observation(0x7190_8086),
        )
        .unwrap();
    topology.seal().unwrap();
    assert!(topology.contains_function(host));
    assert!(!topology.contains_requester(host));
}

#[test]
fn a_failed_quiesced_observation_revokes_the_seal_and_retains_its_error() {
    let isa = topology_address(0, 7, 0);
    let mut topology = PciTopology::new();
    topology
        .observe_quiesced_requester(
            topology_entry_fence().take_quiesced_requester().unwrap(),
            topology_isa_observation(0x7110_8086),
        )
        .unwrap();
    topology.seal().unwrap();
    assert!(topology.contains_requester(isa));
    assert!(topology.quiesced_requester(isa).is_some());
    let error = TopologyError::QuiescedRequesterChanged { at: isa };
    assert_eq!(
        topology.observe_quiesced_requester(
            topology_entry_fence().take_quiesced_requester().unwrap(),
            topology_isa_observation(0x7110_8087),
        ),
        Err(error)
    );
    assert_topology_poisoned(&mut topology, isa, error);
    assert_eq!(
        topology.observe_quiesced_requester(
            topology_entry_fence().take_quiesced_requester().unwrap(),
            topology_isa_observation(0x7110_8086),
        ),
        Err(error)
    );
    assert_eq!(
        topology.observe_non_requester(
            topology_entry_fence().take_non_requester().unwrap(),
            topology_host_observation(0x7190_8087),
        ),
        Err(error),
        "another observation door must preserve the first error"
    );
    topology.reset();
    topology
        .observe_quiesced_requester(
            topology_entry_fence().take_quiesced_requester().unwrap(),
            topology_isa_observation(0x7110_8086),
        )
        .unwrap();
    topology.seal().unwrap();
    assert!(topology.contains_requester(isa));
    assert!(topology.quiesced_requester(isa).is_some());
}

#[test]
fn delegated_non_requester_errors_stay_poisoned_until_reset() {
    let host = topology_address(0, 0, 0);
    let mut observation = topology_host_observation(0x7190_8086);
    observation.header = bridge_header();
    let mut topology = PciTopology::new();
    let error = TopologyError::BridgeIdentity { at: host };
    assert_eq!(
        topology.observe_non_requester(
            topology_entry_fence().take_non_requester().unwrap(),
            observation,
        ),
        Err(error),
    );
    assert_topology_poisoned(&mut topology, host, error);
    assert_eq!(
        topology.observe_non_requester(
            topology_entry_fence().take_non_requester().unwrap(),
            topology_host_observation(0x7190_8086),
        ),
        Err(error),
    );
    topology.reset();
    topology
        .observe_non_requester(
            topology_entry_fence().take_non_requester().unwrap(),
            topology_host_observation(0x7190_8086),
        )
        .unwrap();
    topology.seal().unwrap();
    assert!(topology.contains_function(host));
    assert!(!topology.contains_requester(host));
}

#[test]
fn delegated_quiesced_requester_errors_stay_poisoned_until_reset() {
    let isa = topology_address(0, 7, 0);
    let mut observation = topology_isa_observation(0x7110_8086);
    observation.header = bridge_header();
    let mut topology = PciTopology::new();
    let error = TopologyError::BridgeIdentity { at: isa };
    assert_eq!(
        topology.observe_quiesced_requester(
            topology_entry_fence().take_quiesced_requester().unwrap(),
            observation,
        ),
        Err(error),
    );
    assert_topology_poisoned(&mut topology, isa, error);
    assert_eq!(
        topology.observe_quiesced_requester(
            topology_entry_fence().take_quiesced_requester().unwrap(),
            topology_isa_observation(0x7110_8086),
        ),
        Err(error),
    );
    topology.reset();
    topology
        .observe_quiesced_requester(
            topology_entry_fence().take_quiesced_requester().unwrap(),
            topology_isa_observation(0x7110_8086),
        )
        .unwrap();
    topology.seal().unwrap();
    assert!(topology.contains_requester(isa));
    assert!(topology.quiesced_requester(isa).is_some());
}

#[test]
fn a_sealed_topology_binds_presence_ranges_and_ancestry() {
    let root = topology_address(0, 0x1c, 0);
    let child = topology_address(0x20, 0, 0);
    let root_device = topology_address(0x20, 3, 0);
    let child_device = topology_address(0x30, 4, 1);
    let mut topology = PciTopology::new();
    topology
        .observe(
            root,
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0, 0x20, 0x3f)),
        )
        .expect("root bridge");
    topology
        .observe(
            child,
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0x20, 0x30, 0x35)),
        )
        .expect("child bridge");
    for at in [root_device, child_device] {
        topology
            .observe(at, endpoint_class(), endpoint_header(), None)
            .expect("endpoint");
    }

    topology.seal().expect("rooted topology");
    assert!(topology.is_sealed());
    assert!(topology.contains_function(child_device));
    assert!(!topology.contains_function(topology_address(0x30, 4, 0)));
    assert_eq!(
        topology.bridge_range(root),
        Some(BridgeRange {
            secondary: 0x20,
            subordinate: 0x3f,
        })
    );
    assert!(topology.hierarchy_contains(root, root_device));
    assert!(topology.hierarchy_contains(root, child_device));
    assert!(topology.hierarchy_contains(child, child_device));
    assert!(!topology.hierarchy_contains(child, root_device));
}

#[test]
fn duplicate_unrooted_and_contradictory_functions_refuse_topology() {
    let first = topology_address(0, 0x1c, 0);
    let second = topology_address(0, 0x1d, 0);
    let mut duplicate = PciTopology::new();
    duplicate
        .observe(
            first,
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0, 0x20, 0x20)),
        )
        .expect("first parent");
    assert!(matches!(
        duplicate.observe(
            second,
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0, 0x20, 0x20)),
        ),
        Err(TopologyError::DuplicateParent { bus: 0x20, .. })
    ));

    let mut unrooted = PciTopology::new();
    let endpoint = topology_address(0x40, 2, 0);
    unrooted
        .observe(endpoint, endpoint_class(), endpoint_header(), None)
        .expect("unrooted observation");
    assert_eq!(
        unrooted.seal(),
        Err(TopologyError::UnrootedFunction { at: endpoint })
    );

    let mut contradictory = PciTopology::new();
    assert_eq!(
        contradictory.observe(first, bridge_class(), endpoint_header(), None),
        Err(TopologyError::BridgeIdentity { at: first })
    );
}

#[test]
fn unreadable_and_invalid_bridge_bus_numbers_refuse_topology() {
    let bridge = topology_address(0x20, 0x1c, 0);
    for (numbers, expected) in [
        (None, TopologyError::MissingBusNumbers { at: bridge }),
        (
            Some(bus_numbers(0x21, 0x30, 0x3f)),
            TopologyError::InvalidRange {
                at: bridge,
                primary: 0x21,
                secondary: 0x30,
                subordinate: 0x3f,
            },
        ),
        (
            Some(bus_numbers(0x20, 0x20, 0x3f)),
            TopologyError::InvalidRange {
                at: bridge,
                primary: 0x20,
                secondary: 0x20,
                subordinate: 0x3f,
            },
        ),
        (
            Some(bus_numbers(0x20, 0x30, 0x2f)),
            TopologyError::InvalidRange {
                at: bridge,
                primary: 0x20,
                secondary: 0x30,
                subordinate: 0x2f,
            },
        ),
    ] {
        assert_eq!(
            PciTopology::new().observe(bridge, bridge_class(), bridge_header(), numbers),
            Err(expected)
        );
    }
}

#[test]
fn child_escape_and_unrelated_overlap_refuse_topology() {
    let outer = topology_address(0, 0x1c, 0);
    let child = topology_address(0x20, 0, 0);
    let mut escaped = PciTopology::new();
    escaped
        .observe(
            outer,
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0, 0x20, 0x2f)),
        )
        .expect("outer bridge");
    escaped
        .observe(
            child,
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0x20, 0x30, 0x40)),
        )
        .expect("locally forward child");
    assert_eq!(
        escaped.seal(),
        Err(TopologyError::ChildEscapesParent {
            child,
            parent: outer,
        })
    );

    let sibling = topology_address(0, 0x1d, 0);
    let mut overlap = PciTopology::new();
    overlap
        .observe(
            outer,
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0, 0x20, 0x3f)),
        )
        .expect("first root bridge");
    overlap
        .observe(
            sibling,
            bridge_class(),
            bridge_header(),
            Some(bus_numbers(0, 0x30, 0x35)),
        )
        .expect("second root bridge");
    assert_eq!(
        overlap.seal(),
        Err(TopologyError::UnrelatedOverlap {
            first: outer,
            second: sibling,
        })
    );
}
