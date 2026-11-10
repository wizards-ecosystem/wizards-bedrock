use super::*;

#[derive(Clone, Copy, Debug)]
struct MemoryKindCase {
    kind: MemoryKind,
    name: &'static str,
    allocatable: bool,
    firmware_table: bool,
    direct_mapped: bool,
    device_mapping: bool,
}

fn memory_kind_cases() -> [MemoryKindCase; 9] {
    [
        MemoryKindCase {
            kind: MemoryKind::Usable,
            name: "usable",
            allocatable: true,
            firmware_table: false,
            direct_mapped: true,
            device_mapping: false,
        },
        MemoryKindCase {
            kind: MemoryKind::BootloaderReclaimable,
            name: "bootloader reclaimable",
            allocatable: false,
            firmware_table: false,
            direct_mapped: true,
            device_mapping: false,
        },
        MemoryKindCase {
            kind: MemoryKind::KernelAndModules,
            name: "kernel and modules",
            allocatable: false,
            firmware_table: false,
            direct_mapped: true,
            device_mapping: false,
        },
        MemoryKindCase {
            kind: MemoryKind::Framebuffer,
            name: "framebuffer",
            allocatable: false,
            firmware_table: false,
            direct_mapped: true,
            device_mapping: false,
        },
        MemoryKindCase {
            kind: MemoryKind::AcpiReclaimable,
            name: "acpi reclaimable",
            allocatable: false,
            firmware_table: true,
            direct_mapped: true,
            device_mapping: false,
        },
        MemoryKindCase {
            kind: MemoryKind::AcpiNvs,
            name: "acpi nvs",
            allocatable: false,
            firmware_table: true,
            direct_mapped: true,
            device_mapping: false,
        },
        MemoryKindCase {
            kind: MemoryKind::Reserved,
            name: "reserved",
            allocatable: false,
            firmware_table: false,
            direct_mapped: false,
            device_mapping: true,
        },
        MemoryKindCase {
            kind: MemoryKind::BadMemory,
            name: "bad memory",
            allocatable: false,
            firmware_table: false,
            direct_mapped: false,
            device_mapping: false,
        },
        MemoryKindCase {
            kind: MemoryKind::Unknown(41),
            name: "unknown type 41",
            allocatable: false,
            firmware_table: false,
            direct_mapped: false,
            device_mapping: true,
        },
    ]
}

fn regions(count: usize) -> impl ExactSizeIterator<Item = MemoryRegion> {
    (0..count).map(|index| MemoryRegion {
        base: (index as u64 + 1) * PAGE_SIZE,
        len: PAGE_SIZE,
        kind: MemoryKind::Reserved,
    })
}

#[test]
fn every_memory_kind_has_its_exact_policy_and_name() {
    for case in memory_kind_cases() {
        assert_eq!(case.kind.to_string(), case.name);
        assert_eq!(
            case.kind.is_allocatable(),
            case.allocatable,
            "{:?}",
            case.kind
        );
        assert_eq!(
            case.kind.is_firmware_table(),
            case.firmware_table,
            "{:?}",
            case.kind
        );
        assert_eq!(
            case.kind.is_direct_mapped(),
            case.direct_mapped,
            "{:?}",
            case.kind
        );
        assert_eq!(
            case.kind.permits_device_mapping(),
            case.device_mapping,
            "{:?}",
            case.kind
        );
    }
}

#[test]
fn map_errors_name_the_reported_and_supported_region_counts() {
    assert_eq!(MemoryMapError::Empty.to_string(), "the map is empty");
    assert_eq!(
        MemoryMapError::TooMany {
            observed: 129,
            capacity: 128,
        }
        .to_string(),
        "the map has at least 129 regions but storage holds 128"
    );
    assert_eq!(
        MemoryMapError::InvalidRegion {
            base: u64::MAX,
            len: 1,
        }
        .to_string(),
        "memory region 0xffffffffffffffff + 0x1 overflows"
    );
    assert_eq!(
        MemoryMapError::PageEndOverflow {
            base: u64::MAX - 1,
            len: 1,
        }
        .to_string(),
        "direct-mapped region 0xfffffffffffffffe + 0x1 cannot cover its final page"
    );
}

#[test]
fn debug_reports_only_the_complete_region_count() {
    let map = MemoryMap::try_from_regions([
        MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE * 2,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: PAGE_SIZE * 3,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        },
    ])
    .expect("map");
    assert_eq!(format!("{map:?}"), "MemoryMap { regions: 2 }");
}

/// Transcribed from `covers`'s "input order is not assumed" contract.
#[test]
fn overlapping_regions_answer_the_same_in_either_order() {
    let usable = MemoryRegion {
        base: 0,
        len: PAGE_SIZE * 3,
        kind: MemoryKind::Usable,
    };
    let reserved = MemoryRegion {
        base: 0,
        len: PAGE_SIZE * 2,
        kind: MemoryKind::Reserved,
    };
    let forward = MemoryMap::try_from_regions([reserved, usable]).expect("map");
    let reversed = MemoryMap::try_from_regions([usable, reserved]).expect("map");

    for (start, end) in [(0, 1), (0, PAGE_SIZE), (PAGE_SIZE, PAGE_SIZE * 3)] {
        assert_eq!(
            forward.covers(start, end),
            reversed.covers(start, end),
            "covers({start:#x}, {end:#x}) depends on region order"
        );
        assert_eq!(
            forward.covers_memory(start, end),
            reversed.covers_memory(start, end),
            "covers_memory({start:#x}, {end:#x}) depends on region order"
        );
    }
    assert!(!forward.covers_memory(0, 1));
    assert!(forward.covers(0, 1));
}

#[test]
fn allocatable_runs_remove_a_nested_reserved_frame_in_either_order() {
    let usable = MemoryRegion {
        base: PAGE_SIZE,
        len: PAGE_SIZE * 4,
        kind: MemoryKind::Usable,
    };
    let reserved = MemoryRegion {
        base: PAGE_SIZE * 2,
        len: PAGE_SIZE,
        kind: MemoryKind::Reserved,
    };
    let expected = [
        AllocatableRun {
            base: PAGE_SIZE,
            len: PAGE_SIZE,
        },
        AllocatableRun {
            base: PAGE_SIZE * 3,
            len: PAGE_SIZE * 2,
        },
    ];

    for regions in [[usable, reserved], [reserved, usable]] {
        let map = MemoryMap::try_from_regions(regions).expect("overlapping map");
        assert_eq!(map.allocatable_runs().collect::<Vec<_>>(), expected);
    }
}

#[test]
fn allocatable_runs_exclude_partial_edges_and_every_poisoned_frame() {
    let regions = [
        MemoryRegion {
            base: PAGE_SIZE + 1,
            len: PAGE_SIZE * 8 - 2,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: PAGE_SIZE * 3 + 1,
            len: PAGE_SIZE - 2,
            kind: MemoryKind::Reserved,
        },
        MemoryRegion {
            base: PAGE_SIZE * 5,
            len: PAGE_SIZE,
            kind: MemoryKind::BadMemory,
        },
        MemoryRegion {
            base: PAGE_SIZE * 6 + 1,
            len: 1,
            kind: MemoryKind::Unknown(7),
        },
    ];
    let expected = [
        AllocatableRun {
            base: PAGE_SIZE * 2,
            len: PAGE_SIZE,
        },
        AllocatableRun {
            base: PAGE_SIZE * 4,
            len: PAGE_SIZE,
        },
        AllocatableRun {
            base: PAGE_SIZE * 7,
            len: PAGE_SIZE,
        },
    ];

    let forward = MemoryMap::try_from_regions(regions).expect("map");
    let mut reversed = regions;
    reversed.reverse();
    let reverse = MemoryMap::try_from_regions(reversed).expect("reversed map");
    assert_eq!(forward.allocatable_runs().collect::<Vec<_>>(), expected);
    assert_eq!(reverse.allocatable_runs().collect::<Vec<_>>(), expected);
}

#[test]
fn page_runs_are_disjoint_order_independent_and_count_each_page_once() {
    let usable = MemoryRegion {
        base: PAGE_SIZE,
        len: PAGE_SIZE * 4,
        kind: MemoryKind::Usable,
    };
    let framebuffer = MemoryRegion {
        base: PAGE_SIZE * 2,
        len: PAGE_SIZE * 2,
        kind: MemoryKind::Framebuffer,
    };
    let reserved = MemoryRegion {
        base: PAGE_SIZE * 3,
        len: PAGE_SIZE,
        kind: MemoryKind::Reserved,
    };
    let expected = [
        PageRun {
            base: PAGE_SIZE,
            len: PAGE_SIZE,
            cache: DirectMapCache::Normal,
            allocatable: true,
        },
        PageRun {
            base: PAGE_SIZE * 2,
            len: PAGE_SIZE,
            cache: DirectMapCache::Framebuffer,
            allocatable: false,
        },
        PageRun {
            base: PAGE_SIZE * 4,
            len: PAGE_SIZE,
            cache: DirectMapCache::Normal,
            allocatable: true,
        },
    ];

    for regions in [
        [usable, usable, framebuffer, reserved],
        [reserved, framebuffer, usable, usable],
    ] {
        let map = MemoryMap::try_from_regions(regions).expect("overlapping map");
        let runs = map.page_runs().collect::<Vec<_>>();
        assert_eq!(runs, expected);
        assert_eq!(
            runs.iter().map(|run| run.bytes()).sum::<u64>(),
            PAGE_SIZE * 3
        );
    }
}

#[test]
fn a_boot_range_has_one_direct_map_cache_type_or_none() {
    let map = MemoryMap::try_from_regions([
        MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE * 2,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: PAGE_SIZE * 3,
            len: PAGE_SIZE * 2,
            kind: MemoryKind::Framebuffer,
        },
        MemoryRegion {
            base: PAGE_SIZE * 6,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        },
    ])
    .expect("disjoint map");

    assert_eq!(
        map.direct_map_cache(PAGE_SIZE, PAGE_SIZE * 3),
        Some(DirectMapCache::Normal)
    );
    assert_eq!(
        map.direct_map_cache(PAGE_SIZE * 3, PAGE_SIZE * 5),
        Some(DirectMapCache::Framebuffer)
    );
    assert_eq!(
        map.direct_map_cache(PAGE_SIZE * 2, PAGE_SIZE * 4),
        None,
        "one mapping cannot give ordinary memory and a framebuffer one type"
    );
    assert_eq!(map.direct_map_cache(PAGE_SIZE * 5, PAGE_SIZE * 6), None);
    assert_eq!(map.direct_map_cache(PAGE_SIZE * 6, PAGE_SIZE * 7), None);
}

#[test]
fn page_runs_require_complete_coverage_but_adjacent_claims_may_supply_it() {
    let partial = MemoryMap::try_from_regions([MemoryRegion {
        base: PAGE_SIZE + 1,
        len: 1,
        kind: MemoryKind::Usable,
    }])
    .expect("partial claim");
    assert_eq!(core::iter::Iterator::next(&mut partial.page_runs()), None);

    let hole = MemoryMap::try_from_regions([
        MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE / 2,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: PAGE_SIZE * 2 - PAGE_SIZE / 4,
            len: PAGE_SIZE / 4,
            kind: MemoryKind::AcpiNvs,
        },
    ])
    .expect("claims with a hole");
    assert_eq!(core::iter::Iterator::next(&mut hole.page_runs()), None);

    let adjacent = MemoryMap::try_from_regions([
        MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE / 2,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: PAGE_SIZE + PAGE_SIZE / 2,
            len: PAGE_SIZE / 2,
            kind: MemoryKind::Framebuffer,
        },
    ])
    .expect("adjacent claims");
    assert_eq!(
        adjacent.page_runs().collect::<Vec<_>>(),
        [PageRun {
            base: PAGE_SIZE,
            len: PAGE_SIZE,
            cache: DirectMapCache::Framebuffer,
            allocatable: false,
        }]
    );
}

#[test]
fn a_disallowed_region_at_the_exclusive_end_does_not_overlap() {
    let start = PAGE_SIZE;
    let end = PAGE_SIZE * 3;
    let map = MemoryMap::try_from_regions([
        MemoryRegion {
            base: start,
            len: end - start,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: end,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        },
    ])
    .expect("adjacent regions");

    assert!(map.covers_memory(start, end));
    assert!(!map.covers_memory(start, end + 1));
}

#[test]
fn device_eligibility_covers_the_whole_mapped_page_not_only_register_bytes() {
    let page = PAGE_SIZE;
    let map = MemoryMap::try_from_regions([
        MemoryRegion {
            base: page,
            len: 4,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: page + 4,
            len: page - 4,
            kind: MemoryKind::Reserved,
        },
    ])
    .expect("one split page");

    assert!(map.allows_device_mapping(page + 4, page + 8));
    assert!(!map.allows_device_mapping(page, page * 2));
    assert!(map.allows_device_mapping(page * 2, page * 3));
    assert!(!map.allows_device_mapping(page * 3, page * 3));
}

#[test]
fn every_forbidden_kind_and_byte_in_a_page_refuses_device_mapping() {
    let page = PAGE_SIZE;
    for case in memory_kind_cases()
        .into_iter()
        .filter(|case| !case.device_mapping)
    {
        for offset in 0..PAGE_SIZE {
            let map = MemoryMap::try_from_regions([MemoryRegion {
                base: page + offset,
                len: 1,
                kind: case.kind,
            }])
            .expect("one forbidden byte");
            assert!(
                !map.allows_device_mapping(page, page * 2),
                "{:?} byte {offset:#x} was admitted",
                case.kind
            );
        }
    }

    for case in memory_kind_cases()
        .into_iter()
        .filter(|case| case.device_mapping)
    {
        let map = MemoryMap::try_from_regions([MemoryRegion {
            base: page,
            len: page,
            kind: case.kind,
        }])
        .expect("one device-hole page");
        assert!(map.allows_device_mapping(page, page * 2), "{:?}", case.kind);
    }
}

#[test]
fn a_disallowed_region_at_a_device_extents_exclusive_end_is_disjoint() {
    let map = MemoryMap::try_from_regions([MemoryRegion {
        base: PAGE_SIZE * 2,
        len: PAGE_SIZE,
        kind: MemoryKind::Usable,
    }])
    .expect("map");

    assert!(map.allows_device_mapping(PAGE_SIZE, PAGE_SIZE * 2));
    assert!(!map.allows_device_mapping(PAGE_SIZE, PAGE_SIZE * 2 + 1));
}

/// Memory of kind `around`, then a hole of `kind`, then `around` again.
fn hole_between(kind: MemoryKind, around: MemoryKind) -> MemoryMap {
    MemoryMap::try_from_regions([
        MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE * 2,
            kind: around,
        },
        MemoryRegion {
            base: PAGE_SIZE * 3,
            len: PAGE_SIZE * 2,
            kind,
        },
        MemoryRegion {
            base: PAGE_SIZE * 5,
            len: PAGE_SIZE * 2,
            kind: around,
        },
    ])
    .expect("a hole between two regions")
}

#[test]
fn a_reserved_region_inside_a_device_hole_is_admitted() {
    for case in memory_kind_cases()
        .into_iter()
        .filter(|case| case.device_mapping)
    {
        let map = hole_between(case.kind, MemoryKind::Usable);
        assert!(
            map.allows_reserved_region(PAGE_SIZE * 3, PAGE_SIZE * 5 - 1),
            "{:?}",
            case.kind
        );
        assert!(
            map.allows_reserved_region(PAGE_SIZE * 4, PAGE_SIZE * 4),
            "{:?} one byte",
            case.kind
        );
    }
}

#[test]
fn a_reserved_region_one_byte_into_kernel_memory_is_refused() {
    for case in memory_kind_cases()
        .into_iter()
        .filter(|case| !case.device_mapping)
    {
        let map = hole_between(MemoryKind::Reserved, case.kind);
        assert!(
            map.allows_reserved_region(PAGE_SIZE * 3, PAGE_SIZE * 5 - 1),
            "{:?} around the hole",
            case.kind
        );
        assert!(
            !map.allows_reserved_region(PAGE_SIZE * 3 - 1, PAGE_SIZE * 5 - 1),
            "{:?} below the hole",
            case.kind
        );
        assert!(
            !map.allows_reserved_region(PAGE_SIZE * 3, PAGE_SIZE * 5),
            "{:?} above the hole",
            case.kind
        );
    }
}

#[test]
fn a_reserved_region_sharing_a_page_with_kernel_memory_is_refused() {
    let half = PAGE_SIZE / 2;
    for case in memory_kind_cases()
        .into_iter()
        .filter(|case| !case.device_mapping)
    {
        let below = MemoryMap::try_from_regions([
            MemoryRegion {
                base: PAGE_SIZE,
                len: PAGE_SIZE * 2 + half,
                kind: case.kind,
            },
            MemoryRegion {
                base: PAGE_SIZE * 3 + half,
                len: PAGE_SIZE * 2 - half,
                kind: MemoryKind::Reserved,
            },
        ])
        .expect("kernel memory ending inside a page");
        assert!(
            below.allows_reserved_region(PAGE_SIZE * 4, PAGE_SIZE * 5 - 1),
            "{:?} below a whole page",
            case.kind
        );
        assert!(
            !below.allows_reserved_region(PAGE_SIZE * 3 + half, PAGE_SIZE * 5 - 1),
            "{:?} in the first page",
            case.kind
        );

        let above = MemoryMap::try_from_regions([
            MemoryRegion {
                base: PAGE_SIZE * 3,
                len: PAGE_SIZE + half,
                kind: MemoryKind::Reserved,
            },
            MemoryRegion {
                base: PAGE_SIZE * 4 + half,
                len: PAGE_SIZE * 2,
                kind: case.kind,
            },
        ])
        .expect("kernel memory starting inside a page");
        assert!(
            above.allows_reserved_region(PAGE_SIZE * 3, PAGE_SIZE * 4 - 1),
            "{:?} above a whole page",
            case.kind
        );
        assert!(
            !above.allows_reserved_region(PAGE_SIZE * 3, PAGE_SIZE * 4 + half - 1),
            "{:?} in the last page",
            case.kind
        );
    }
}

#[test]
fn an_empty_or_inverted_reserved_region_is_refused() {
    let map = MemoryMap::try_from_regions([MemoryRegion {
        base: PAGE_SIZE,
        len: PAGE_SIZE * 8,
        kind: MemoryKind::Reserved,
    }])
    .expect("one reserved span");
    let middle = PAGE_SIZE * 3 + PAGE_SIZE / 2;
    assert!(map.allows_reserved_region(middle, middle));
    for (base, limit) in [
        (middle, middle - 1),
        (middle + 1, middle - 1),
        (PAGE_SIZE * 4, PAGE_SIZE * 3),
    ] {
        assert!(
            !map.allows_reserved_region(base, limit),
            "[{base:#x}, {limit:#x}]"
        );
    }
}

#[test]
fn a_named_memory_run_is_memory_by_the_other_contract() {
    let usable = MemoryRegion {
        base: 0,
        len: PAGE_SIZE * 3,
        kind: MemoryKind::Usable,
    };
    let reserved = MemoryRegion {
        base: 0,
        len: PAGE_SIZE * 2,
        kind: MemoryKind::Reserved,
    };
    let map = MemoryMap::try_from_regions([reserved, usable]).expect("map");

    for len in [1, PAGE_SIZE, PAGE_SIZE * 2] {
        if let Some(base) = map.first_memory_run(len) {
            let end = base.checked_add(len).expect("a run inside the space");
            assert!(
                map.covers_memory(base, end),
                "first_memory_run({len:#x}) named {base:#x}, which covers_memory denies"
            );
        }
    }
}

#[test]
fn the_first_run_jumps_past_a_reserved_prefix() {
    let map = MemoryMap::try_from_regions([
        MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE * 3,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        },
    ])
    .expect("overlapping map");

    assert_eq!(map.first_memory_run(PAGE_SIZE), Some(PAGE_SIZE * 2));
}

#[test]
fn a_zero_length_run_has_no_first_byte() {
    let map = MemoryMap::try_from_regions([MemoryRegion {
        base: PAGE_SIZE,
        len: PAGE_SIZE,
        kind: MemoryKind::Usable,
    }])
    .expect("map");

    assert_eq!(map.first_memory_run(0), None);
}

#[test]
fn an_overflowing_region_refuses_the_whole_map() {
    let valid = MemoryRegion {
        base: PAGE_SIZE,
        len: PAGE_SIZE,
        kind: MemoryKind::Usable,
    };
    let overflowing = MemoryRegion {
        base: u64::MAX,
        len: 1,
        kind: MemoryKind::Reserved,
    };

    assert_eq!(
        MemoryMap::try_from_regions([valid, overflowing]),
        Err(MemoryMapError::InvalidRegion {
            base: u64::MAX,
            len: 1,
        })
    );
    assert!(
        MemoryMap::try_from_regions([MemoryRegion {
            base: u64::MAX,
            len: 0,
            kind: MemoryKind::Reserved,
        }])
        .is_ok()
    );
    assert_eq!(
        MemoryMap::try_from_regions([MemoryRegion {
            base: u64::MAX - 1,
            len: 1,
            kind: MemoryKind::Usable,
        }]),
        Err(MemoryMapError::PageEndOverflow {
            base: u64::MAX - 1,
            len: 1,
        })
    );
}

#[test]
fn a_zero_length_region_does_not_occupy_its_address() {
    let map = MemoryMap::try_from_regions([MemoryRegion {
        base: 0x2000,
        len: 0,
        kind: MemoryKind::Usable,
    }])
    .expect("a zero-length region is representable");

    assert!(
        map.allows_device_mapping(0x1000, 0x3000),
        "an empty usable region blocked a range that contains no byte of it"
    );
}

#[test]
fn an_inverted_span_is_not_covered() {
    let map = MemoryMap::try_from_regions([MemoryRegion {
        base: 0,
        len: PAGE_SIZE * 4,
        kind: MemoryKind::Usable,
    }])
    .expect("a usable run");

    assert!(!map.covers_memory(0x3000, 0x1000));
    assert_eq!(map.direct_map_cache(0x3000, 0x1000), None);
}

#[test]
fn the_region_limit_is_all_or_nothing() {
    assert_eq!(
        MemoryMap::try_from_regions(core::iter::empty()),
        Err(MemoryMapError::Empty)
    );
    for count in [MAX_REGIONS - 1, MAX_REGIONS] {
        let map = MemoryMap::try_from_regions(regions(count)).expect("map within bound");
        assert_eq!(map.regions().count(), count);
        assert!(map.regions().eq(regions(count)));
    }

    assert_eq!(
        MemoryMap::try_from_regions(regions(MAX_REGIONS + 1)),
        Err(MemoryMapError::TooMany {
            observed: MAX_REGIONS + 1,
            capacity: MAX_REGIONS,
        })
    );
}

#[test]
fn an_overfull_producer_is_not_polled_past_the_first_refused_region() {
    #[derive(Debug)]
    struct BoundedProducer {
        next: usize,
    }

    impl Iterator for BoundedProducer {
        type Item = MemoryRegion;

        fn next(&mut self) -> Option<Self::Item> {
            assert!(
                self.next <= MAX_REGIONS,
                "constructor polled past its bound"
            );
            let index = self.next;
            self.next += 1;
            Some(MemoryRegion {
                base: index as u64 * PAGE_SIZE,
                len: PAGE_SIZE,
                kind: MemoryKind::Reserved,
            })
        }
    }

    assert_eq!(
        MemoryMap::try_from_regions(BoundedProducer { next: 0 }),
        Err(MemoryMapError::TooMany {
            observed: MAX_REGIONS + 1,
            capacity: MAX_REGIONS,
        })
    );
}

#[test]
fn every_accepted_region_informs_memory_policy() {
    let tail = 0x4000_0000;
    let mut input = regions(MAX_REGIONS).collect::<Vec<_>>();
    input[MAX_REGIONS - 3] = MemoryRegion {
        base: tail,
        len: PAGE_SIZE * 2,
        kind: MemoryKind::Usable,
    };
    input[MAX_REGIONS - 2] = MemoryRegion {
        base: tail + PAGE_SIZE * 2,
        len: PAGE_SIZE,
        kind: MemoryKind::AcpiNvs,
    };
    input[MAX_REGIONS - 1] = MemoryRegion {
        base: tail + PAGE_SIZE * 3,
        len: PAGE_SIZE,
        kind: MemoryKind::Reserved,
    };

    let map = MemoryMap::try_from_regions(input).expect("map at bound");
    assert_eq!(map.first_memory_run(PAGE_SIZE * 2), Some(tail));
    assert!(map.covers(tail, tail + PAGE_SIZE * 4));
    assert!(map.covers_memory(tail, tail + PAGE_SIZE * 3));
    assert!(!map.covers_memory(tail, tail + PAGE_SIZE * 4));
}
