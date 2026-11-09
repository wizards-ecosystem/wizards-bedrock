use super::*;

/// Bit positions, restated from Intel SDM Vol. 3A table 4-19 rather than
/// from the constants above. A transcription error shows up here as a
/// failure; copying the constants would only prove they equal themselves.
#[test]
fn flag_bits_match_the_manual() {
    assert_eq!(Flags::PRESENT.bits(), 1);
    assert_eq!(Flags::WRITABLE.bits(), 2);
    assert_eq!(Flags::USER.bits(), 4);
    assert_eq!(Flags::WRITE_THROUGH.bits(), 8);
    assert_eq!(Flags::NO_CACHE.bits(), 16);
    assert_eq!(Flags::ACCESSED.bits(), 32);
    assert_eq!(Flags::DIRTY.bits(), 64);
    assert_eq!(Flags::HUGE.bits(), 128);
    assert_eq!(Flags::GLOBAL.bits(), 256);
    assert_eq!(Flags::NO_EXECUTE.bits(), 0x8000_0000_0000_0000);
}

#[test]
fn the_address_field_is_bits_51_through_12() {
    assert_eq!(ADDRESS_MASK.trailing_zeros(), 12);
    assert_eq!(ADDRESS_MASK.count_ones(), 40, "51 - 12 + 1 bits");
    assert_eq!(ADDRESS_MASK.leading_zeros(), 12, "bits 63:52 are flags");
}

#[test]
fn union_is_idempotent_when_the_sets_overlap() {
    assert_eq!(Flags::PRESENT.union(Flags::PRESENT), Flags::PRESENT);
    assert_eq!(
        Flags::PRESENT.union(Flags::WRITABLE).union(Flags::PRESENT),
        Flags::PRESENT.union(Flags::WRITABLE)
    );
}

#[test]
fn page_identity_ignores_only_hardware_observation_bits() {
    let expected = Flags::PRESENT
        .union(Flags::WRITABLE)
        .union(Flags::NO_EXECUTE)
        .union(Flags::NO_CACHE);
    for observed in [
        Flags::NONE,
        Flags::ACCESSED,
        Flags::DIRTY,
        Flags::ACCESSED.union(Flags::DIRTY),
    ] {
        assert!(page_identity_matches(
            0x3000,
            expected.union(observed),
            PAGE_SIZE,
            0x3000,
            expected,
        ));
    }

    for (mapped, flags, span) in [
        (0x4000, expected, PAGE_SIZE),
        (0x3000, expected.difference(Flags::NO_CACHE), PAGE_SIZE),
        (0x3000, expected.union(Flags::WRITE_THROUGH), PAGE_SIZE),
        (0x3000, expected, LARGE_PAGE_SIZE),
    ] {
        assert!(!page_identity_matches(
            mapped, flags, span, 0x3000, expected,
        ));
    }
}

#[test]
fn table_identity_ignores_only_accessed() {
    let expected = Entry::table(0x3000, TablePrivilege::Supervisor).unwrap();
    let accessed = Entry::from_bits(expected.bits() | Flags::ACCESSED.bits());
    assert!(expected.table_identity_matches(accessed));
    assert!(accessed.table_identity_matches(expected));

    for bit in 0..u64::BITS {
        if bit == Flags::ACCESSED.bits().trailing_zeros() {
            continue;
        }
        let drifted = Entry::from_bits(expected.bits() ^ (1_u64 << bit));
        assert!(
            !expected.table_identity_matches(drifted),
            "bit {bit} was omitted from table identity"
        );
    }
}

#[test]
fn an_entry_round_trips_its_address_and_flags() {
    let flags = Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE;
    let entry = Entry::page(Level::Pt, 0x1234_5000, flags).expect("aligned");

    assert_eq!(entry.address(), 0x1234_5000);
    assert_eq!(entry.flags(), flags);
    assert!(entry.is_present());
    assert_eq!(Entry::from_bits(entry.bits()), entry);
}

#[test]
fn the_address_and_the_flags_never_overlap() {
    // The highest representable address alongside every flag: neither may
    // corrupt the other.
    let flags = Flags::PRESENT | Flags::NO_EXECUTE | Flags::GLOBAL;
    let top = MAX_PHYSICAL - PAGE_SIZE;
    let entry = Entry::page(Level::Pt, top, flags).expect("in range");

    assert_eq!(entry.address(), top);
    assert_eq!(entry.flags(), flags);
}

#[test]
fn a_misaligned_or_oversized_address_is_refused() {
    assert_eq!(
        Entry::page(Level::Pt, 0x1001, Flags::PRESENT),
        Err(EntryError::Misaligned(0x1001))
    );
    assert_eq!(
        Entry::page(Level::Pt, MAX_PHYSICAL, Flags::PRESENT),
        Err(EntryError::OutOfRange(MAX_PHYSICAL))
    );
}

#[test]
fn an_empty_entry_is_absent() {
    assert!(!Entry::ABSENT.is_present());
    assert_eq!(Entry::ABSENT.address(), 0);
    assert_eq!(Entry::ABSENT.flags(), Flags::NONE);
}

#[test]
fn a_supervisor_table_does_not_grant_user_access() {
    let entry = Entry::table(0x3000, TablePrivilege::Supervisor).expect("aligned");

    assert!(entry.is_present());
    assert!(entry.flags().contains(Flags::WRITABLE));
    assert!(
        !entry.flags().contains(Flags::NO_EXECUTE),
        "permissions intersect down the walk, so a parent must not restrict"
    );
    assert!(!entry.flags().contains(Flags::USER));
}

#[test]
fn a_user_table_does_not_block_user_leaves() {
    let user = Entry::table(0x3000, TablePrivilege::User).expect("aligned");
    assert!(user.flags().contains(Flags::PRESENT));
    assert!(user.flags().contains(Flags::WRITABLE));
    assert!(user.flags().contains(Flags::USER));
}

#[test]
fn write_execute_is_exactly_writable_executable_and_present() {
    assert!((Flags::PRESENT | Flags::WRITABLE).is_write_execute());
    assert!(
        !(Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE).is_write_execute(),
        "no-execute is what makes a writable page safe"
    );
    assert!(
        !Flags::PRESENT.is_write_execute(),
        "read-only and executable is the other half of W^X"
    );
    assert!(
        !(Flags::WRITABLE).is_write_execute(),
        "an absent entry maps nothing, so it violates nothing"
    );
}

#[test]
fn level_shifts_are_nine_bits_apart_above_the_page_offset() {
    assert_eq!(Level::Pt.shift(), 12);
    assert_eq!(Level::Pd.shift(), 21);
    assert_eq!(Level::Pdpt.shift(), 30);
    assert_eq!(Level::Pml4.shift(), 39);
}

#[test]
fn a_level_spans_what_its_shift_says() {
    assert_eq!(Level::Pt.span(), PAGE_SIZE);
    assert_eq!(Level::Pd.span(), LARGE_PAGE_SIZE);
    assert_eq!(Level::Pdpt.span(), 1 << 30);
    assert_eq!(Level::Pml4.span(), 512 << 30);
}

#[test]
fn indices_decompose_a_known_address() {
    // The kernel's own base. Worked by hand from the bit layout:
    // 0xffff_ffff_8000_0000 >> 39 = ...1ff, and the rest follow.
    let base = 0xffff_ffff_8000_0000;
    assert_eq!(Level::Pml4.index(base), 511);
    assert_eq!(Level::Pdpt.index(base), 510);
    assert_eq!(Level::Pd.index(base), 0);
    assert_eq!(Level::Pt.index(base), 0);
}

#[test]
fn every_index_stays_inside_a_table() {
    for level in Level::WALK {
        assert_eq!(level.index(u64::MAX), ENTRIES - 1);
        assert_eq!(level.index(0), 0);
        // One past this level's span wraps to the next slot, never out.
        assert_eq!(level.index(level.span()), 1);
        assert_eq!(level.index(level.span() * ENTRIES as u64), 0);
    }
}

#[test]
fn the_walk_descends_one_level_at_a_time() {
    assert_eq!(
        Level::WALK,
        [Level::Pml4, Level::Pdpt, Level::Pd, Level::Pt]
    );

    for pair in Level::WALK.windows(2) {
        assert!(pair[0] > pair[1], "the walk goes downward");
    }
}

#[test]
fn only_the_top_level_cannot_map_a_page() {
    assert!(!Level::Pml4.can_map_page());
    assert!(Level::Pdpt.can_map_page());
    assert!(Level::Pd.can_map_page());
    assert!(Level::Pt.can_map_page());
}

#[test]
fn a_mapping_range_preserves_checked_bounds() {
    let range = MappingRange::new(
        0xffff_8000_0000_0000,
        MAX_PHYSICAL - 2 * PAGE_SIZE,
        2 * PAGE_SIZE,
    )
    .expect("valid range");
    assert_eq!(range.virtual_base(), 0xffff_8000_0000_0000);
    assert_eq!(range.physical_base(), MAX_PHYSICAL - 2 * PAGE_SIZE);
    assert_eq!(range.len(), 2 * PAGE_SIZE);

    let low_edge = MappingRange::new(0x0000_7fff_ffff_f000, 0, PAGE_SIZE);
    assert!(low_edge.is_ok(), "the last canonical low page is mappable");
}

#[test]
fn an_empty_mapping_range_is_refused() {
    assert_eq!(
        MappingRange::new(0, 0, 0),
        Err(MappingRangeError::ZeroLength)
    );
}

#[test]
fn a_misaligned_virtual_mapping_base_is_refused() {
    assert_eq!(
        MappingRange::new(1, 0, PAGE_SIZE),
        Err(MappingRangeError::MisalignedVirtualBase(1))
    );
}

#[test]
fn a_misaligned_physical_mapping_base_is_refused() {
    assert_eq!(
        MappingRange::new(0, 1, PAGE_SIZE),
        Err(MappingRangeError::MisalignedPhysicalBase(1))
    );
}

#[test]
fn a_partial_page_mapping_length_is_refused() {
    assert_eq!(
        MappingRange::new(0, 0, PAGE_SIZE + 1),
        Err(MappingRangeError::MisalignedLength(PAGE_SIZE + 1))
    );
}

#[test]
fn a_virtual_mapping_end_that_wraps_is_refused() {
    assert_eq!(
        MappingRange::new(u64::MAX - (PAGE_SIZE - 1), 0, 2 * PAGE_SIZE),
        Err(MappingRangeError::VirtualEndOverflow)
    );
}

#[test]
fn a_physical_mapping_end_that_wraps_is_refused() {
    assert_eq!(
        MappingRange::new(0, u64::MAX - (PAGE_SIZE - 1), 2 * PAGE_SIZE),
        Err(MappingRangeError::PhysicalEndOverflow)
    );
}

#[test]
fn a_noncanonical_mapping_start_is_refused() {
    let start = 0x0000_8000_0000_0000;
    assert_eq!(
        MappingRange::new(start, 0, PAGE_SIZE),
        Err(MappingRangeError::NonCanonicalVirtual(start))
    );
}

#[test]
fn a_noncanonical_mapping_end_is_refused() {
    let last = 0x0000_8000_0000_0fff;
    assert_eq!(
        MappingRange::new(0x0000_7fff_ffff_f000, 0, 2 * PAGE_SIZE),
        Err(MappingRangeError::NonCanonicalVirtual(last))
    );
}

#[test]
fn a_mapping_with_canonical_endpoints_that_crosses_the_hole_is_refused() {
    assert_eq!(
        MappingRange::new(0, 0, 0xffff_8000_0000_1000),
        Err(MappingRangeError::CrossesCanonicalHole)
    );
}

#[test]
fn a_physical_mapping_start_outside_the_entry_encoding_is_refused() {
    assert_eq!(
        MappingRange::new(0, MAX_PHYSICAL, PAGE_SIZE),
        Err(MappingRangeError::PhysicalOutOfRange(MAX_PHYSICAL))
    );
}

#[test]
fn a_physical_mapping_end_outside_the_entry_encoding_is_refused() {
    let last = MAX_PHYSICAL + PAGE_SIZE - 1;
    assert_eq!(
        MappingRange::new(0, MAX_PHYSICAL - PAGE_SIZE, 2 * PAGE_SIZE),
        Err(MappingRangeError::PhysicalOutOfRange(last))
    );
}

#[test]
fn canonical_addresses_are_sign_extended_from_bit_47() {
    assert!(is_canonical(0));
    assert!(is_canonical(0x0000_7fff_ffff_ffff), "top of the low half");
    assert!(is_canonical(0xffff_8000_0000_0000), "base of the high half");
    assert!(is_canonical(0xffff_ffff_8000_0000), "the kernel's own base");

    assert!(!is_canonical(0x0000_8000_0000_0000), "the hole below");
    assert!(!is_canonical(0xffff_7fff_ffff_ffff), "the hole above");
}

#[test]
fn no_user_mapping_reaches_the_last_canonical_page() {
    assert_eq!(USER_END, 0x0000_8000_0000_0000);
    assert!(!is_canonical(USER_END), "the hole begins at USER_END");
    assert_eq!(USER_MAPPING_END, 0x0000_7fff_ffff_f000);

    assert!(user_mapping_fits(0, PAGE_SIZE));
    assert!(user_mapping_fits(0, USER_MAPPING_END));
    assert!(user_mapping_fits(USER_MAPPING_END - PAGE_SIZE, PAGE_SIZE));
    assert!(user_mapping_fits(USER_MAPPING_END, 0));

    assert!(!user_mapping_fits(USER_MAPPING_END, PAGE_SIZE));
    assert!(!user_mapping_fits(
        USER_MAPPING_END - PAGE_SIZE,
        2 * PAGE_SIZE
    ));
    assert!(!user_mapping_fits(USER_MAPPING_END - 1, 2));
    assert!(!user_mapping_fits(0, USER_END));
    assert!(!user_mapping_fits(u64::MAX, 1));

    // The last byte a trap can follow is USER_MAPPING_END - 1, so the
    // saved return address is at most USER_MAPPING_END, still canonical.
    assert!(is_canonical(USER_MAPPING_END));
}

#[test]
fn canonical_address_admission_covers_both_sides_of_the_hole() {
    const LOW_END: u64 = 0x0000_7fff_ffff_ffff;
    const HIGH_START: u64 = 0xffff_8000_0000_0000;

    assert_eq!(CanonicalAddress::ZERO.get(), 0);
    for address in [0, LOW_END, HIGH_START, u64::MAX] {
        assert_eq!(
            CanonicalAddress::new(address).map(CanonicalAddress::get),
            Some(address)
        );
    }
    for address in [LOW_END + 1, 0x7fff_ffff_ffff_ffff, HIGH_START - 1] {
        assert_eq!(CanonicalAddress::new(address), None, "{address:#x}");
    }
}

#[test]
fn a_huge_entry_is_distinguishable_from_a_table_pointer() {
    let huge = Entry::page(Level::Pd, 0x20_0000, Flags::PRESENT).expect("aligned");
    let table = Entry::table(0x20_0000, TablePrivilege::Supervisor).expect("aligned");

    assert!(huge.is_huge());
    assert!(!table.is_huge());
    assert_eq!(
        huge.address(),
        table.address(),
        "same target, different kind"
    );
}

#[test]
fn a_page_table_leaf_uses_four_kib_alignment_without_the_huge_bit() {
    let leaf = Entry::page(Level::Pt, 0x1000, Flags::PRESENT).expect("4 KiB aligned");
    assert!(!leaf.flags().contains(Flags::HUGE));
}

#[test]
fn a_page_directory_leaf_uses_two_mib_alignment_and_the_huge_bit() {
    let leaf = Entry::page(Level::Pd, LARGE_PAGE_SIZE, Flags::PRESENT).expect("2 MiB aligned");
    assert!(leaf.flags().contains(Flags::HUGE));
    assert_eq!(
        Entry::page(Level::Pd, 0x1000, Flags::PRESENT),
        Err(EntryError::Misaligned(0x1000))
    );
}

#[test]
fn a_pdpt_leaf_uses_one_gib_alignment_and_the_huge_bit() {
    assert_eq!(
        Entry::page(Level::Pdpt, LARGE_PAGE_SIZE, Flags::PRESENT),
        Err(EntryError::Misaligned(LARGE_PAGE_SIZE))
    );

    let one_gib = Entry::page(Level::Pdpt, 1 << 30, Flags::PRESENT).expect("1 GiB aligned");
    assert!(one_gib.flags().contains(Flags::HUGE));
}

#[test]
fn a_caller_cannot_supply_the_level_derived_huge_flag() {
    assert_eq!(
        Entry::page(Level::Pt, 0x1000, Flags::PRESENT | Flags::HUGE),
        Err(EntryError::HugeFlagIsDerived)
    );
}

#[test]
fn a_top_level_entry_cannot_be_a_leaf() {
    assert_eq!(
        Entry::page(Level::Pml4, 0, Flags::PRESENT),
        Err(EntryError::CannotMapPage(Level::Pml4))
    );
}

#[test]
fn debug_says_what_the_entry_is() {
    extern crate std;
    use std::format;

    assert_eq!(format!("{:?}", Entry::ABSENT), "Entry(absent)");
    let entry =
        Entry::page(Level::Pt, 0x1000, Flags::PRESENT | Flags::NO_EXECUTE).expect("aligned");
    assert_eq!(format!("{entry:?}"), "Entry(0x1000 present no-execute)");
    assert_eq!(
        format!("{:?}", Flags::PRESENT | Flags::WRITABLE | Flags::OWNED),
        "present writable owned"
    );
    assert_eq!(format!("{:?}", Flags::NONE), "none");
}

#[test]
fn entry_errors_name_the_refused_value_or_rule() {
    let cases = [
        (EntryError::Misaligned(0x1001), "0x1001 is not page-aligned"),
        (
            EntryError::OutOfRange(0x1234),
            "0x1234 does not fit 52 bits",
        ),
        (
            EntryError::CannotMapPage(Level::Pml4),
            "Pml4 cannot map a page",
        ),
        (
            EntryError::HugeFlagIsDerived,
            "the huge-page flag is derived from the leaf level",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn mapping_range_errors_name_the_refused_value_or_rule() {
    let cases = [
        (MappingRangeError::ZeroLength, "mapping length is zero"),
        (
            MappingRangeError::MisalignedVirtualBase(0x1001),
            "virtual base 0x1001 is not page-aligned",
        ),
        (
            MappingRangeError::MisalignedPhysicalBase(0x2001),
            "physical base 0x2001 is not page-aligned",
        ),
        (
            MappingRangeError::MisalignedLength(0x3001),
            "mapping length 12289 is not page-aligned",
        ),
        (
            MappingRangeError::VirtualEndOverflow,
            "virtual mapping end overflows",
        ),
        (
            MappingRangeError::PhysicalEndOverflow,
            "physical mapping end overflows",
        ),
        (
            MappingRangeError::NonCanonicalVirtual(0x8000_0000_0000),
            "virtual address 0x800000000000 is not canonical",
        ),
        (
            MappingRangeError::CrossesCanonicalHole,
            "virtual mapping crosses the noncanonical hole",
        ),
        (
            MappingRangeError::PhysicalOutOfRange(0x10_0000_0000_0000),
            "physical address 0x10000000000000 does not fit 52 bits",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

/// The owned bit is software-only: the CPU never sees it, and it survives
/// a round trip through an entry.
///
/// It sits in the architecture's `AVL` window (bits 9..11), clear of every
/// flag the hardware defines and clear of the address field, so setting it
/// changes neither where a mapping points nor what it permits.
#[test]
fn the_owned_bit_is_software_only_and_survives_an_entry() {
    const AVL: u64 = 0b111 << 9;
    assert_eq!(Flags::OWNED.bits() & AVL, Flags::OWNED.bits());

    for hardware in [
        Flags::PRESENT,
        Flags::WRITABLE,
        Flags::USER,
        Flags::WRITE_THROUGH,
        Flags::NO_CACHE,
        Flags::ACCESSED,
        Flags::DIRTY,
        Flags::HUGE,
        Flags::GLOBAL,
        Flags::NO_EXECUTE,
    ] {
        assert_eq!(
            Flags::OWNED.bits() & hardware.bits(),
            0,
            "the owned bit overlaps a hardware flag"
        );
    }

    let address = 0x1234_5000;
    let flags = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::OWNED);
    let entry = Entry::page(Level::Pt, address, flags).expect("a page entry");
    assert_eq!(entry.address(), address, "the owned bit moved the address");
    assert!(entry.flags().contains(Flags::OWNED));
    assert!(entry.is_present());

    let unowned =
        Entry::page(Level::Pt, address, flags.difference(Flags::OWNED)).expect("a page entry");
    assert!(!unowned.flags().contains(Flags::OWNED));
    assert_eq!(
        entry.address(),
        unowned.address(),
        "ownership changed where the mapping points"
    );
}

/// An absent entry maps nothing and permits nothing.
#[test]
fn an_absent_entry_is_present_to_nobody() {
    assert!(!Entry::ABSENT.is_present());
    assert!(!Entry::ABSENT.is_huge());
    assert_eq!(Entry::ABSENT.address(), 0);
    assert_eq!(Entry::ABSENT.flags().bits(), 0);
    assert!(!Entry::ABSENT.flags().contains(Flags::OWNED));
}

/// Intersection keeps what both hold, and nothing else.
///
/// The operation a walk uses to narrow a permission at every level, so it
/// is worth its own check: an implementation that returned the union, or
/// either operand, would make a walk report a permission the CPU does not
/// grant.
#[test]
fn intersection_keeps_only_what_both_hold() {
    let user_write = Flags::USER.union(Flags::WRITABLE);
    assert_eq!(user_write.intersection(Flags::USER), Flags::USER);
    assert_eq!(
        user_write.intersection(Flags::NONE).bits(),
        0,
        "intersecting with nothing kept something"
    );
    assert_eq!(user_write.intersection(user_write), user_write);
    assert_eq!(
        Flags::USER.intersection(Flags::WRITABLE).bits(),
        0,
        "disjoint flags intersected to something"
    );

    // And it narrows: the result is contained in both operands.
    let left = Flags::USER.union(Flags::NO_EXECUTE).union(Flags::PRESENT);
    let right = Flags::USER.union(Flags::WRITABLE);
    let both = left.intersection(right);
    assert!(left.contains(both) && right.contains(both));
    assert_eq!(both, Flags::USER);
}

#[test]
fn a_narrower_parent_beats_a_wider_leaf() {
    let supervisor = Flags::PRESENT.union(Flags::WRITABLE);
    let read_only = Flags::PRESENT.union(Flags::USER);
    let leaf = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::USER);

    let under_supervisor = Flags::EVERYTHING_ALLOWED.through(supervisor).through(leaf);
    assert!(!under_supervisor.effective_for(leaf).contains(Flags::USER));
    assert!(
        under_supervisor
            .effective_for(leaf)
            .contains(Flags::WRITABLE)
    );

    let under_read_only = Flags::EVERYTHING_ALLOWED.through(read_only).through(leaf);
    assert!(
        !under_read_only
            .effective_for(leaf)
            .contains(Flags::WRITABLE)
    );
    assert!(under_read_only.effective_for(leaf).contains(Flags::USER));
}

#[test]
fn no_execute_anywhere_on_the_path_forbids_execution() {
    let plain = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::USER);
    let nx = plain.union(Flags::NO_EXECUTE);

    for path in [[nx, plain, plain], [plain, nx, plain], [plain, plain, nx]] {
        let mut walked = Flags::EVERYTHING_ALLOWED;
        for entry in path {
            walked = walked.through(entry);
        }
        let leaf = *path.last().expect("three entries");
        assert!(walked.effective_for(leaf).contains(Flags::NO_EXECUTE));
    }

    let mut clean = Flags::EVERYTHING_ALLOWED;
    for entry in [plain, plain, plain] {
        clean = clean.through(entry);
    }
    assert!(!clean.effective_for(plain).contains(Flags::NO_EXECUTE));
}

#[test]
fn a_walk_that_never_narrows_reports_the_leaf_unchanged() {
    let wide = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::USER);
    for leaf in [
        Flags::PRESENT,
        Flags::PRESENT.union(Flags::NO_EXECUTE),
        Flags::PRESENT.union(Flags::OWNED),
        Flags::PRESENT.union(Flags::HUGE).union(Flags::WRITABLE),
        wide,
    ] {
        let walked = Flags::EVERYTHING_ALLOWED.through(wide).through(leaf);
        assert_eq!(walked.effective_for(leaf), leaf, "leaf {leaf:?}");
    }
}

#[test]
fn narrowing_never_grants_what_the_leaf_withholds() {
    let wide = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::USER);
    let leaf = Flags::PRESENT;
    let walked = Flags::EVERYTHING_ALLOWED.through(wide).through(leaf);
    let effective = walked.effective_for(leaf);
    assert!(!effective.contains(Flags::USER));
    assert!(!effective.contains(Flags::WRITABLE));
}
