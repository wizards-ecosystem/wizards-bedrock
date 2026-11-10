use super::*;

use crate::requesters::{MAX_DRHD_UNITS, MAX_REQUESTER_SCOPES};

#[test]
fn a_dmar_yields_its_structures_and_resolves_each_bdf_to_its_unit_and_regions() {
    let scoped_target = pci_endpoint_scope(2, 3, 0);
    let rmrr_scope = pci_endpoint_scope(2, 3, 0);
    let mut structures = Vec::new();
    structures.extend_from_slice(&drhd_bytes(true, 0, 0xfed9_0000, &[]));
    structures.extend_from_slice(&drhd_bytes(false, 0, 0xfed9_1000, &scoped_target));
    structures.extend_from_slice(&rmrr_bytes(0, 0x1000, 0x1fff, &rmrr_scope));
    structures.extend_from_slice(&atsr_bytes(0, &[]));
    structures.extend_from_slice(&rhsa_bytes(0xfed9_1000, 1));
    let bytes = table(&Sdt::DMAR, &dmar_body(&structures));

    let parsed: Vec<_> = DmarEntries::new(&bytes).expect("validated dmar").collect();
    assert_eq!(
        parsed,
        [
            DmarStructure::Drhd {
                include_pci_all: true,
                segment: 0,
                register_base: 0xfed9_0000,
                scopes: &[],
            },
            DmarStructure::Drhd {
                include_pci_all: false,
                segment: 0,
                register_base: 0xfed9_1000,
                scopes: &scoped_target,
            },
            DmarStructure::Rmrr {
                segment: 0,
                base: 0x1000,
                limit: 0x1fff,
                scopes: &rmrr_scope,
            },
            DmarStructure::Atsr {
                segment: 0,
                scopes: &[],
            },
            DmarStructure::Rhsa {
                register_base: 0xfed9_1000,
                proximity_domain: 1,
            },
        ]
    );

    let dmar = Dmar::new(&bytes).expect("no duplicate units, no inverted regions");
    assert_eq!(dmar.unit_count(), 2);
    assert!(!dmar.interrupt_remapping());
    assert!(!dmar.x2apic_opt_out());
    let scoped_bdf = Bdf::new(0, 2, 3, 0);
    let unscoped_bdf = Bdf::new(0, 9, 0, 0);
    assert_eq!(
        dmar.unit_for(scoped_bdf, no_bridges, no_hierarchies),
        Some(GoverningUnit {
            register_base: 0xfed9_1000,
            coverage: Coverage::Endpoint,
        })
    );
    assert_eq!(
        dmar.unit_for(unscoped_bdf, no_bridges, no_hierarchies),
        Some(GoverningUnit {
            register_base: 0xfed9_0000,
            coverage: Coverage::CatchAll,
        })
    );
    assert_eq!(
        dmar.reserved_regions_for(scoped_bdf, no_bridges, no_hierarchies)
            .collect::<Vec<_>>(),
        [(0x1000, 0x1fff)]
    );
    assert_eq!(
        dmar.reserved_regions_for(unscoped_bdf, no_bridges, no_hierarchies)
            .collect::<Vec<_>>(),
        []
    );
}

const SCOPED: u64 = 0xfed9_1000;
const INNER: u64 = 0xfed9_2000;
const EXACT: u64 = 0xfed9_3000;
const CATCH_ALL: u64 = 0xfed9_0000;

fn buses(secondary: u8, subordinate: u8) -> BusRange {
    BusRange {
        secondary,
        subordinate,
    }
}

/// A topology in which only the listed functions are bridges.
fn topology(bridges: &[(Bdf, BusRange)]) -> impl Fn(Bdf) -> Option<BusRange> + '_ {
    move |at| {
        bridges
            .iter()
            .find(|(bridge, _)| *bridge == at)
            .map(|(_, behind)| *behind)
    }
}

/// The unique direct-parent chain represented by the listed bridges.
fn hierarchy(bridges: &[(Bdf, BusRange)]) -> impl Fn(Bdf, Bdf) -> bool + '_ {
    move |root, target| {
        if root == target {
            return true;
        }
        let mut bus = target.bus();
        for _ in 0..u8::MAX {
            let mut parents = bridges.iter().filter(|(_, range)| range.secondary == bus);
            let Some((parent, _)) = parents.next() else {
                return false;
            };
            if parents.next().is_some() {
                return false;
            }
            if *parent == root {
                return true;
            }
            bus = parent.bus();
            if bus == 0 {
                return false;
            }
        }
        false
    }
}

fn no_bridges(_: Bdf) -> Option<BusRange> {
    None
}

fn no_hierarchies(_: Bdf, _: Bdf) -> bool {
    false
}

fn no_requesters(_: Bdf) -> bool {
    false
}

fn present(requesters: &[Bdf]) -> impl Fn(Bdf) -> bool + '_ {
    move |at| requesters.contains(&at)
}

fn dmar_of(structures: &[Vec<u8>]) -> Vec<u8> {
    table(&Sdt::DMAR, &dmar_body(&structures.concat()))
}

const fn catch_all() -> GoverningUnit {
    GoverningUnit {
        register_base: CATCH_ALL,
        coverage: Coverage::CatchAll,
    }
}

#[test]
fn an_endpoint_inventory_owns_each_distinct_endpoint_on_the_unit() {
    let first = Bdf::new(0, 0, 4, 0);
    let second = Bdf::new(0, 0, 5, 0);
    let first_scope = scope(ENDPOINT, 0, &[(4, 0)]);
    let second_scope = scope(ENDPOINT, 0, &[(5, 0)]);
    let scopes = [
        first_scope.as_slice(),
        second_scope.as_slice(),
        first_scope.as_slice(),
        scope(IO_APIC, 0, &[(0x1f, 0)]).as_slice(),
    ]
    .concat();
    let bytes = dmar_of(&[drhd_bytes(false, 0, SCOPED, &scopes)]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    let inventory = dmar
        .requester_inventory::<2>(second, no_bridges, no_hierarchies, no_requesters)
        .expect("bounded requester inventory");
    assert_eq!(inventory.unit().register_base, SCOPED);
    assert_eq!(inventory.len(), 2);
    assert!(!inventory.is_empty());
    assert_eq!(inventory.iter().collect::<Vec<_>>(), [first, second]);
}

#[test]
fn requester_inventory_expands_hierarchies_catch_all_and_refuses_overflow() {
    let selected = Bdf::new(0, 0, 4, 0);
    let endpoint = scope(ENDPOINT, 0, &[(4, 0)]);

    let include_all = dmar_of(&[drhd_bytes(true, 0, SCOPED, &endpoint)]);
    let inventory = Dmar::new(&include_all)
        .expect("valid dmar")
        .requester_inventory::<2>(selected, no_bridges, no_hierarchies, present(&[selected]))
        .expect("a sole complete unit has a bounded segment inventory");
    assert_eq!(inventory.iter().collect::<Vec<_>>(), [selected]);

    let bridge = Bdf::new(0, 0, 0x1c, 0);
    let bridges = [(bridge, buses(0x20, 0x20))];
    let downstream = Bdf::new(0, 0x20, 3, 1);
    let hierarchy_scope = scope(SUB_HIERARCHY, 0, &[(0x1c, 0)]);
    let shared = dmar_of(&[drhd_bytes(
        false,
        0,
        SCOPED,
        &[endpoint.as_slice(), hierarchy_scope.as_slice()].concat(),
    )]);
    let inventory = Dmar::new(&shared)
        .expect("valid dmar")
        .requester_inventory::<3>(
            selected,
            topology(&bridges),
            hierarchy(&bridges),
            present(&[downstream]),
        )
        .expect("bounded hierarchy inventory");
    assert_eq!(
        inventory.iter().collect::<Vec<_>>(),
        [selected, bridge, downstream]
    );

    let crowded = dmar_of(&[drhd_bytes(
        false,
        0,
        SCOPED,
        &[
            endpoint.as_slice(),
            scope(ENDPOINT, 0, &[(5, 0)]).as_slice(),
        ]
        .concat(),
    )]);
    assert_eq!(
        Dmar::new(&crowded)
            .expect("valid dmar")
            .requester_inventory::<1>(selected, no_bridges, no_hierarchies, no_requesters,),
        Err(RequesterInventoryError::TooManyRequesters { maximum: 1 })
    );
}

#[test]
fn requester_inventory_refuses_unresolved_foreign_and_catch_all_selection() {
    let selected = Bdf::new(0, 0, 4, 0);
    let endpoint = scope(ENDPOINT, 0, &[(4, 0)]);
    let unresolved = scope(ENDPOINT, 0, &[(0x1c, 0), (5, 0)]);
    let bytes = dmar_of(&[drhd_bytes(
        false,
        0,
        SCOPED,
        &[endpoint.as_slice(), unresolved.as_slice()].concat(),
    )]);
    assert_eq!(
        Dmar::new(&bytes)
            .expect("valid dmar")
            .requester_inventory::<2>(selected, no_bridges, no_hierarchies, no_requesters,),
        Err(RequesterInventoryError::UnresolvedScope)
    );

    let sibling = scope(ENDPOINT, 0, &[(5, 0)]);
    let ambiguous = dmar_of(&[
        drhd_bytes(
            false,
            0,
            SCOPED,
            &[endpoint.as_slice(), sibling.as_slice()].concat(),
        ),
        drhd_bytes(false, 0, EXACT, &sibling),
    ]);
    assert_eq!(
        Dmar::new(&ambiguous)
            .expect("valid dmar")
            .requester_inventory::<2>(selected, no_bridges, no_hierarchies, no_requesters,),
        Err(RequesterInventoryError::ForeignEndpoint)
    );

    let catch_all = dmar_of(&[drhd_bytes(true, 0, CATCH_ALL, &[])]);
    assert_eq!(
        Dmar::new(&catch_all)
            .expect("valid dmar")
            .requester_inventory::<2>(
                Bdf::new(1, 0, 4, 0),
                no_bridges,
                no_hierarchies,
                no_requesters,
            ),
        Err(RequesterInventoryError::Ungoverned)
    );
}

#[test]
fn catch_all_inventory_enumerates_every_present_requester_with_a_fixed_budget() {
    let selected = Bdf::new(0, 0, 4, 0);
    let sibling = Bdf::new(0, 0x20, 3, 1);
    let catch_all_bytes = dmar_of(&[drhd_bytes(true, 0, CATCH_ALL, &[])]);
    let dmar = Dmar::new(&catch_all_bytes).expect("valid catch-all dmar");

    let inventory = dmar
        .requester_inventory::<2>(
            selected,
            no_bridges,
            no_hierarchies,
            present(&[selected, sibling]),
        )
        .expect("bounded catch-all inventory");
    assert_eq!(inventory.unit(), catch_all());
    assert_eq!(inventory.iter().collect::<Vec<_>>(), [selected, sibling]);
    assert_eq!(
        dmar.requester_inventory::<1>(
            selected,
            no_bridges,
            no_hierarchies,
            present(&[selected, sibling]),
        ),
        Err(RequesterInventoryError::TooManyRequesters { maximum: 1 })
    );
}

#[test]
fn a_selected_requester_under_a_hierarchy_is_inventoried_with_its_bridge() {
    let bridge = Bdf::new(0, 0, 0x1c, 0);
    let selected = Bdf::new(0, 0x20, 3, 0);
    let bridges = [(bridge, buses(0x20, 0x20))];
    let bytes = dmar_of(&[drhd_bytes(
        false,
        0,
        SCOPED,
        &scope(SUB_HIERARCHY, 0, &[(0x1c, 0)]),
    )]);

    let inventory = Dmar::new(&bytes)
        .expect("valid dmar")
        .requester_inventory::<2>(
            selected,
            topology(&bridges),
            hierarchy(&bridges),
            present(&[selected]),
        )
        .expect("complete hierarchy inventory");
    assert_eq!(inventory.unit().coverage, Coverage::Bridge(bridge));
    assert_eq!(inventory.iter().collect::<Vec<_>>(), [bridge, selected]);
}

#[test]
fn overlapping_sibling_hierarchies_are_ambiguous_not_numerically_nested() {
    let first = Bdf::new(0, 0, 0x1c, 0);
    let second = Bdf::new(0, 0, 0x1d, 0);
    let target = Bdf::new(0, 0x30, 3, 0);
    let bridges = [(first, buses(0x20, 0x3f)), (second, buses(0x30, 0x4f))];
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &scope(SUB_HIERARCHY, 0, &[(0x1c, 0)])),
        drhd_bytes(false, 0, INNER, &scope(SUB_HIERARCHY, 0, &[(0x1d, 0)])),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    let contradictory =
        |root, descendant| descendant == target && (root == first || root == second);
    assert_eq!(
        dmar.unit_for(target, topology(&bridges), contradictory),
        None
    );
    assert_eq!(
        dmar.requester_inventory::<4>(
            first,
            topology(&bridges),
            contradictory,
            present(&[target]),
        ),
        Err(RequesterInventoryError::AmbiguousRequester)
    );
}

#[test]
fn a_numeric_bus_range_without_ancestry_names_no_requester_or_reserved_region() {
    let bridge = Bdf::new(0, 0, 0x1c, 0);
    let target = Bdf::new(0, 0x30, 3, 0);
    let bridges = [(bridge, buses(0x20, 0x3f))];
    let hierarchy_scope = scope(SUB_HIERARCHY, 0, &[(0x1c, 0)]);
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &hierarchy_scope),
        drhd_bytes(true, 0, CATCH_ALL, &[]),
        rmrr_bytes(0, 0x7000, 0x7fff, &hierarchy_scope),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    assert_eq!(
        dmar.unit_for(target, topology(&bridges), no_hierarchies),
        Some(catch_all())
    );
    assert_eq!(
        dmar.reserved_regions_for(target, topology(&bridges), no_hierarchies)
            .count(),
        0
    );
}

#[test]
fn requester_scope_work_is_bounded_before_duplicate_hierarchy_scans() {
    let selected = Bdf::new(0, 0, 4, 0);
    let bridge = Bdf::new(0, 0, 0x1c, 0);
    let endpoint = scope(ENDPOINT, 0, &[(4, 0)]);
    let hierarchy_scope = scope(SUB_HIERARCHY, 0, &[(0x1c, 0)]);
    let mut scopes = endpoint;
    for _ in 0..MAX_REQUESTER_SCOPES {
        scopes.extend_from_slice(&hierarchy_scope);
    }
    let bytes = dmar_of(&[drhd_bytes(false, 0, SCOPED, &scopes)]);
    let bridges = [(bridge, buses(0x20, 0x20))];

    assert_eq!(
        Dmar::new(&bytes)
            .expect("valid dmar")
            .requester_inventory::<4>(
                selected,
                topology(&bridges),
                hierarchy(&bridges),
                no_requesters,
            ),
        Err(RequesterInventoryError::TooManyRequesterScopes {
            maximum: MAX_REQUESTER_SCOPES,
        })
    );
}

#[test]
fn requester_inventory_refuses_non_pci_and_unknown_requester_scopes() {
    let selected = Bdf::new(0, 0, 4, 0);
    let endpoint = scope(ENDPOINT, 0, &[(4, 0)]);
    for kind in [NAMESPACE_DEVICE, 0x7f] {
        let scopes = [endpoint.as_slice(), scope(kind, 0, &[(0x1f, 0)]).as_slice()].concat();
        let bytes = dmar_of(&[drhd_bytes(false, 0, SCOPED, &scopes)]);
        assert_eq!(
            Dmar::new(&bytes)
                .expect("valid dmar")
                .requester_inventory::<2>(selected, no_bridges, no_hierarchies, no_requesters,),
            Err(RequesterInventoryError::UnsupportedScope)
        );
    }
}

// Every bus number differs, so a walk that stops early or strays misses the target.
#[test]
fn an_endpoint_scope_resolves_through_each_bridge_on_its_path() {
    let root_port = Bdf::new(3, 0x10, 0x1c, 4);
    let switch_port = Bdf::new(3, 0x20, 0x03, 2);
    let bridges = [
        (root_port, buses(0x20, 0x2f)),
        (switch_port, buses(0x24, 0x26)),
    ];
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let endpoint = scope(ENDPOINT, 0x10, &[(0x1c, 4), (0x03, 2), (0x05, 1)]);
    let bytes = dmar_of(&[
        drhd_bytes(false, 3, SCOPED, &endpoint),
        drhd_bytes(true, 3, CATCH_ALL, &[]),
        rmrr_bytes(3, 0x7000, 0x7fff, &endpoint),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    let target = Bdf::new(3, 0x24, 0x05, 1);
    assert_eq!(
        dmar.unit_for(target, &topology, &ancestry),
        Some(GoverningUnit {
            register_base: SCOPED,
            coverage: Coverage::Endpoint,
        })
    );
    assert_eq!(
        dmar.reserved_regions_for(target, &topology, &ancestry)
            .collect::<Vec<_>>(),
        [(0x7000, 0x7fff)]
    );
    for elsewhere in [
        Bdf::new(3, 0x10, 0x05, 1),
        Bdf::new(3, 0x20, 0x05, 1),
        Bdf::new(3, 0x26, 0x05, 1),
        Bdf::new(3, 0x24, 0x05, 0),
    ] {
        assert_eq!(
            dmar.unit_for(elsewhere, &topology, &ancestry),
            Some(catch_all()),
            "{elsewhere:?}"
        );
        assert_eq!(
            dmar.reserved_regions_for(elsewhere, &topology, &ancestry)
                .count(),
            0,
            "{elsewhere:?}"
        );
    }
}

#[test]
fn a_bridge_scope_covers_its_bridge_and_exactly_the_buses_behind_it() {
    let bridge = Bdf::new(0, 0x00, 0x1c, 0);
    let last_bus_bridge = Bdf::new(0, 0x20, 0, 0);
    let bridges = [
        (bridge, buses(0x20, 0x2f)),
        (last_bus_bridge, buses(0x2f, 0x2f)),
    ];
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let hierarchy = scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)]);
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &hierarchy),
        drhd_bytes(true, 0, CATCH_ALL, &[]),
        rmrr_bytes(0, 0x7000, 0x7fff, &hierarchy),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    let behind_bridge = GoverningUnit {
        register_base: SCOPED,
        coverage: Coverage::Bridge(bridge),
    };
    let functions = [
        (bridge, Some(behind_bridge)),
        (Bdf::new(0, 0x1f, 0x05, 1), Some(catch_all())),
        (Bdf::new(0, 0x20, 0x05, 1), Some(behind_bridge)),
        (Bdf::new(0, 0x2f, 0x05, 1), Some(behind_bridge)),
        (Bdf::new(0, 0x30, 0x05, 1), Some(catch_all())),
    ];
    for (function, governed) in functions {
        assert_eq!(
            dmar.unit_for(function, &topology, &ancestry),
            governed,
            "{function:?}"
        );
        let reserved = dmar
            .reserved_regions_for(function, &topology, &ancestry)
            .count();
        assert_eq!(reserved, usize::from(governed == Some(behind_bridge)));
    }
}

#[test]
fn a_path_through_a_function_that_is_not_a_bridge_names_nothing() {
    let through = scope(ENDPOINT, 0x00, &[(0x1c, 0), (0x05, 1)]);
    let hierarchy = scope(SUB_HIERARCHY, 0x00, &[(0x1d, 0)]);
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &[through, hierarchy].concat()),
        drhd_bytes(true, 0, CATCH_ALL, &[]),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    for function in [
        Bdf::new(0, 0x00, 0x05, 1),
        Bdf::new(0, 0x00, 0x1c, 0),
        Bdf::new(0, 0x00, 0x1d, 0),
    ] {
        assert_eq!(
            dmar.unit_for(function, no_bridges, no_hierarchies),
            Some(catch_all()),
            "{function:?}"
        );
    }
}

#[test]
fn a_bridge_that_does_not_lead_past_its_own_bus_names_nothing() {
    let bridges = [
        (Bdf::new(0, 0x04, 0x1c, 0), buses(0x04, 0x08)),
        (Bdf::new(0, 0x30, 0x1c, 0), buses(0x20, 0x2f)),
    ];
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let looping = scope(ENDPOINT, 0x04, &[(0x1c, 0), (0x05, 1)]);
    let backward = scope(ENDPOINT, 0x30, &[(0x1c, 0), (0x06, 1)]);
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &[looping, backward].concat()),
        drhd_bytes(true, 0, CATCH_ALL, &[]),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    for function in [Bdf::new(0, 0x04, 0x05, 1), Bdf::new(0, 0x20, 0x06, 1)] {
        assert_eq!(
            dmar.unit_for(function, &topology, &ancestry),
            Some(catch_all()),
            "{function:?}"
        );
    }
}

#[test]
fn a_bridge_whose_secondary_bus_exceeds_its_subordinate_names_nothing() {
    let bridge = Bdf::new(0, 0x00, 0x1c, 0);
    let bridges = [(bridge, buses(0x30, 0x20))];
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let hierarchy = scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)]);
    let through = scope(ENDPOINT, 0x00, &[(0x1c, 0), (0x05, 1)]);
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &[hierarchy, through].concat()),
        drhd_bytes(true, 0, CATCH_ALL, &[]),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    for function in [
        bridge,
        Bdf::new(0, 0x30, 0x05, 1),
        Bdf::new(0, 0x25, 0x05, 1),
    ] {
        assert_eq!(
            dmar.unit_for(function, &topology, &ancestry),
            Some(catch_all()),
            "{function:?}"
        );
    }
}

// Nine valid hops exceed the retired bridge-path array's old bound.
#[test]
fn a_long_endpoint_path_still_outranks_another_units_outer_hierarchy() {
    const HOPS: usize = 9;
    let bridges = (0..u8::try_from(HOPS - 1).expect("fits"))
        .map(|bus| (Bdf::new(0, bus, 0x01, 0), buses(bus + 1, 8)))
        .collect::<Vec<_>>();
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let mut endpoint_path = vec![(0x01, 0); HOPS - 1];
    endpoint_path.push((0x05, 1));
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, INNER, &scope(SUB_HIERARCHY, 0, &[(0x01, 0)])),
        drhd_bytes(false, 0, EXACT, &scope(ENDPOINT, 0, &endpoint_path)),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");
    let target = Bdf::new(0, 8, 0x05, 1);

    assert_eq!(
        dmar.unit_for(target, &topology, &ancestry),
        Some(GoverningUnit {
            register_base: EXACT,
            coverage: Coverage::Endpoint,
        })
    );
}

// Each carries the path an endpoint scope for the target would.
#[test]
fn scopes_that_name_no_pci_function_never_govern_one() {
    let target = Bdf::new(0, 0x00, 0x1f, 0);
    let foreign = [IO_APIC, HPET, NAMESPACE_DEVICE, 0x7f]
        .map(|kind| scope(kind, 0x00, &[(0x1f, 0)]))
        .concat();
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &foreign),
        drhd_bytes(true, 0, CATCH_ALL, &[]),
        rmrr_bytes(0, 0x7000, 0x7fff, &foreign),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    assert_eq!(
        dmar.unit_for(target, no_bridges, no_hierarchies),
        Some(catch_all())
    );
    assert_eq!(
        dmar.reserved_regions_for(target, no_bridges, no_hierarchies)
            .count(),
        0
    );
}

// An outer bridge, an inner bridge and an endpoint, each on its own unit, listed outermost first.
#[test]
fn the_most_specific_scope_governs_across_units() {
    let outer = Bdf::new(0, 0x00, 0x1c, 0);
    let inner = Bdf::new(0, 0x20, 0x00, 0);
    let bridges = [(outer, buses(0x20, 0x2f)), (inner, buses(0x21, 0x22))];
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)])),
        drhd_bytes(
            false,
            0,
            INNER,
            &scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0), (0x00, 0)]),
        ),
        drhd_bytes(
            false,
            0,
            EXACT,
            &scope(ENDPOINT, 0x00, &[(0x1c, 0), (0x00, 0), (0x00, 0)]),
        ),
        drhd_bytes(true, 0, CATCH_ALL, &[]),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    let governed = [
        (Bdf::new(0, 0x21, 0x00, 0), EXACT, Coverage::Endpoint),
        (Bdf::new(0, 0x21, 0x03, 0), INNER, Coverage::Bridge(inner)),
        (Bdf::new(0, 0x20, 0x05, 0), SCOPED, Coverage::Bridge(outer)),
        (Bdf::new(0, 0x30, 0x00, 0), CATCH_ALL, Coverage::CatchAll),
    ];
    for (function, register_base, coverage) in governed {
        assert_eq!(
            dmar.unit_for(function, &topology, &ancestry),
            Some(GoverningUnit {
                register_base,
                coverage,
            }),
            "{function:?}"
        );
    }
}

// The catch-all is listed first, so table order cannot keep it from a named function.
#[test]
fn the_catch_all_unit_governs_only_functions_no_scope_names() {
    let bridge = Bdf::new(0, 0x00, 0x1c, 0);
    let bridges = [(bridge, buses(0x20, 0x20))];
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let bytes = dmar_of(&[
        drhd_bytes(true, 0, CATCH_ALL, &[]),
        drhd_bytes(false, 0, EXACT, &pci_endpoint_scope(0x00, 0x05, 0)),
        drhd_bytes(false, 0, SCOPED, &scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)])),
        drhd_bytes(false, 2, INNER, &pci_endpoint_scope(0x00, 0x07, 0)),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    let governed = [
        (
            Bdf::new(0, 0x00, 0x05, 0),
            Some((EXACT, Coverage::Endpoint)),
        ),
        (
            Bdf::new(0, 0x20, 0x00, 0),
            Some((SCOPED, Coverage::Bridge(bridge))),
        ),
        (
            Bdf::new(0, 0x00, 0x06, 0),
            Some((CATCH_ALL, Coverage::CatchAll)),
        ),
        (Bdf::new(2, 0x00, 0x08, 0), None),
        (Bdf::new(1, 0x00, 0x05, 0), None),
    ];
    for (function, expected) in governed {
        let expected = expected.map(|(register_base, coverage)| GoverningUnit {
            register_base,
            coverage,
        });
        assert_eq!(
            dmar.unit_for(function, &topology, &ancestry),
            expected,
            "{function:?}"
        );
    }
}

#[test]
fn excessive_and_ambiguous_remapping_authority_fails_closed() {
    let units = (0..=MAX_DRHD_UNITS)
        .map(|index| drhd_bytes(false, 0, 0x1000 + index as u64 * 0x1000, &[]))
        .collect::<Vec<_>>();
    assert_eq!(
        Dmar::new(&dmar_of(&units)),
        Err(DmarError::TooManyUnits {
            maximum: MAX_DRHD_UNITS
        })
    );

    let target = Bdf::new(0, 0x00, 0x05, 0);
    let endpoint = pci_endpoint_scope(0x00, 0x05, 0);
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, EXACT, &endpoint),
        drhd_bytes(false, 0, SCOPED, &endpoint),
        drhd_bytes(true, 0, CATCH_ALL, &[]),
    ]);
    let dmar = Dmar::new(&bytes).expect("structurally valid ambiguous endpoints");
    assert_eq!(dmar.unit_for(target, no_bridges, no_hierarchies), None);

    let elsewhere = Bdf::new(0, 0x00, 0x06, 0);
    let bytes = dmar_of(&[
        drhd_bytes(true, 0, CATCH_ALL, &[]),
        drhd_bytes(true, 0, INNER, &[]),
    ]);
    let dmar = Dmar::new(&bytes).expect("structurally valid ambiguous catch-all units");
    assert_eq!(dmar.unit_for(elsewhere, no_bridges, no_hierarchies), None);

    let bridge = Bdf::new(0, 0x00, 0x1c, 0);
    let bridges = [(bridge, buses(0x20, 0x2f))];
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let hierarchy = scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)]);
    let bytes = dmar_of(&[
        drhd_bytes(false, 0, SCOPED, &hierarchy),
        drhd_bytes(false, 0, INNER, &hierarchy),
    ]);
    let dmar = Dmar::new(&bytes).expect("structurally valid ambiguous bridges");
    assert_eq!(
        dmar.unit_for(Bdf::new(0, 0x20, 0x01, 0), &topology, &ancestry),
        None
    );
}

#[test]
fn a_reserved_region_follows_scopes_resolved_through_the_topology() {
    let bridge = Bdf::new(0, 0x00, 0x1c, 0);
    let child = Bdf::new(0, 0x20, 0x02, 0);
    let bridges = [(bridge, buses(0x20, 0x22)), (child, buses(0x22, 0x22))];
    let topology = topology(&bridges);
    let ancestry = hierarchy(&bridges);
    let bytes = dmar_of(&[
        drhd_bytes(true, 0, CATCH_ALL, &[]),
        rmrr_bytes(
            0,
            0x7000,
            0x7fff,
            &scope(ENDPOINT, 0x00, &[(0x1c, 0), (0x00, 0)]),
        ),
        rmrr_bytes(0, 0x9000, 0x9fff, &scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)])),
    ]);
    let dmar = Dmar::new(&bytes).expect("valid dmar");

    let regions = |function| {
        dmar.reserved_regions_for(function, &topology, &ancestry)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        regions(Bdf::new(0, 0x20, 0x00, 0)),
        [(0x7000, 0x7fff), (0x9000, 0x9fff)]
    );
    assert_eq!(regions(Bdf::new(0, 0x22, 0x01, 0)), [(0x9000, 0x9fff)]);
    assert_eq!(regions(Bdf::new(0, 0x23, 0x00, 0)), []);
}
