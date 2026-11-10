use super::*;

#[test]
fn dmar_platform_flags_decode_interrupt_remapping_and_x2apic_opt_out() {
    for (flags, interrupt_remapping, x2apic_opt_out) in [
        (0, false, false),
        (1, true, false),
        (2, false, true),
        (3, true, true),
    ] {
        let mut body = dmar_body(&[]);
        body[1] = flags;
        let bytes = table(&Sdt::DMAR, &body);
        let dmar = Dmar::new(&bytes).expect("flag bits do not alter structure validity");
        assert_eq!(dmar.interrupt_remapping(), interrupt_remapping);
        assert_eq!(dmar.x2apic_opt_out(), x2apic_opt_out);
    }
}

#[test]
fn a_single_catch_all_drhd_without_rmrr_exposes_its_register_base() {
    let empty = table(&Sdt::DMAR, &dmar_body(&[]));
    let empty = Dmar::new(&empty).expect("empty DMAR");
    assert_eq!(empty.sole_unit_segment(), None);
    assert_eq!(empty.sole_complete_unit_register_base(0), None);

    for segment in [0, 7, u16::MAX] {
        let bytes = table(
            &Sdt::DMAR,
            &dmar_body(&drhd_bytes(true, segment, 0xfed9_0000, &[])),
        );
        let dmar = Dmar::new(&bytes).expect("one DRHD");
        assert_eq!(dmar.sole_unit_segment(), Some(segment));
        assert_eq!(
            dmar.sole_rmrr_free_unit_register_base(segment),
            Some(0xfed9_0000)
        );
        assert_eq!(
            dmar.sole_complete_unit_register_base(segment),
            Some(0xfed9_0000)
        );
        assert_eq!(
            dmar.sole_complete_unit_register_base(segment.wrapping_add(1)),
            None
        );
    }

    let scoped = table(
        &Sdt::DMAR,
        &dmar_body(&drhd_bytes(false, 0, 0xfed9_0000, &[])),
    );
    let scoped = Dmar::new(&scoped).expect("one scoped DRHD");
    assert_eq!(
        scoped.sole_rmrr_free_unit_register_base(0),
        Some(0xfed9_0000)
    );
    assert_eq!(scoped.sole_complete_unit_register_base(0), None);

    let mut reserved = drhd_bytes(true, 0, 0xfed9_0000, &[]);
    reserved.extend_from_slice(&rmrr_bytes(0, 0x1000, 0x1fff, &[]));
    let reserved = table(&Sdt::DMAR, &dmar_body(&reserved));
    let reserved = Dmar::new(&reserved).expect("one catch-all DRHD and one RMRR");
    assert_eq!(reserved.sole_rmrr_free_unit_register_base(0), None);
    assert_eq!(reserved.sole_complete_unit_register_base(0), None);

    let mut multiple = drhd_bytes(true, 0, 0xfed9_0000, &[]);
    multiple.extend_from_slice(&drhd_bytes(true, 9, 0xfed9_1000, &[]));
    let bytes = table(&Sdt::DMAR, &dmar_body(&multiple));
    let multiple = Dmar::new(&bytes).expect("distinct DRHDs");
    assert_eq!(multiple.sole_unit_segment(), None);
    assert_eq!(multiple.sole_rmrr_free_unit_register_base(0), None);
    assert_eq!(multiple.sole_rmrr_free_unit_register_base(9), None);
    assert_eq!(multiple.sole_complete_unit_register_base(0), None);
    assert_eq!(multiple.sole_complete_unit_register_base(9), None);
}

#[test]
fn a_dmar_view_refuses_a_table_that_is_not_a_dmar() {
    let bytes = table(&Sdt::MADT, &[0; 12]);
    assert!(matches!(
        DmarEntries::new(&bytes),
        Err(AcpiError::WrongTable {
            expected: Sdt::DMAR,
            found: Sdt::MADT,
        })
    ));
}

#[test]
fn a_dmar_without_its_fixed_preamble_is_invalid() {
    let bytes = table(&Sdt::DMAR, &[]);
    assert_eq!(
        DmarEntries::new(&bytes).map(|entries| entries.count()),
        Err(AcpiError::BadLength(Sdt::LEN as u32))
    );
}

#[test]
fn a_truncated_dmar_structure_header_is_refused_not_read() {
    let mut body = dmar_body(&[]);
    body.extend_from_slice(&[0, 0, 16]); // an incomplete type+length prefix
    let bytes = table(&Sdt::DMAR, &body);
    assert!(matches!(
        DmarEntries::new(&bytes),
        Err(AcpiError::MalformedEntry { length: 0, .. })
    ));
}

// The corrupt input that would otherwise loop forever.
#[test]
fn a_dmar_structure_claiming_zero_length_invalidates_the_table() {
    let mut body = dmar_body(&[]);
    body.extend_from_slice(&[0, 0, 0, 0]); // kind 0 (DRHD), length 0
    let bytes = table(&Sdt::DMAR, &body);
    assert!(matches!(
        DmarEntries::new(&bytes),
        Err(AcpiError::MalformedEntry { length: 0, .. })
    ));
}

#[test]
fn a_dmar_structure_running_past_the_table_invalidates_it() {
    let mut body = dmar_body(&[]);
    body.extend_from_slice(&[0, 0, 40, 0]); // kind 0, length 40, 4 bytes actually present
    let bytes = table(&Sdt::DMAR, &body);
    assert!(matches!(
        DmarEntries::new(&bytes),
        Err(AcpiError::MalformedEntry {
            length: 40,
            available: 4,
            ..
        })
    ));
}

#[test]
fn a_device_scope_running_past_its_structure_invalidates_the_table() {
    let short_scope = [1u8, 8, 0, 0, 0, 0]; // claims 8 bytes; only 6 are present
    let drhd = drhd_bytes(false, 0, 0x1000, &short_scope);
    let bytes = table(&Sdt::DMAR, &dmar_body(&drhd));
    assert!(matches!(
        DmarEntries::new(&bytes),
        Err(AcpiError::MalformedEntry {
            length: 8,
            available: 6,
            ..
        })
    ));
}

#[test]
fn an_unknown_dmar_structure_is_preserved_as_other_and_does_not_stop_the_walk() {
    let mut structures = vec![9, 0, 8, 0, 1, 2, 3, 4]; // kind 9, length 8
    structures.extend_from_slice(&drhd_bytes(true, 0, 0x2000, &[]));
    let bytes = table(&Sdt::DMAR, &dmar_body(&structures));

    let parsed: Vec<_> = DmarEntries::new(&bytes)
        .expect("unknown kinds still validate")
        .collect();
    assert_eq!(
        parsed,
        [
            DmarStructure::Other { kind: 9 },
            DmarStructure::Drhd {
                include_pci_all: true,
                segment: 0,
                register_base: 0x2000,
                scopes: &[],
            },
        ]
    );
}

// Field widths from the VT-d specification's DRHD, RMRR, ATSR and RHSA layouts.
#[test]
fn a_dmar_structure_shorter_than_its_fields_is_refused_not_read() {
    for (kind, minimum) in [(0u16, 16usize), (1, 24), (2, 8), (3, 20), (0x80, 4)] {
        let short_len = minimum - 1;
        let mut short = vec![0u8; short_len];
        if short_len >= 4 {
            short[0..2].copy_from_slice(&kind.to_le_bytes());
            let len = u16::try_from(short_len).expect("fits");
            short[2..4].copy_from_slice(&len.to_le_bytes());
        }
        let bytes = table(&Sdt::DMAR, &dmar_body(&short));
        assert!(
            DmarEntries::new(&bytes).is_err(),
            "kind {kind} accepted {short_len} bytes, one short of {minimum}"
        );

        let mut exact = vec![0u8; minimum];
        exact[0..2].copy_from_slice(&kind.to_le_bytes());
        let len = u16::try_from(minimum).expect("fits");
        exact[2..4].copy_from_slice(&len.to_le_bytes());
        let bytes = table(&Sdt::DMAR, &dmar_body(&exact));
        assert_eq!(
            DmarEntries::new(&bytes).map(|entries| entries.count()),
            Ok(1),
            "kind {kind} refused exactly {minimum} bytes"
        );
    }
}

// Host address width, flags and ten reserved bytes, from the VT-d specification's DMAR header.
#[test]
fn the_first_remapping_structure_begins_after_the_dmar_preamble() {
    assert_eq!(DmarEntries::HEADER, 48);
    assert_eq!(Sdt::LEN + 1 + 1 + 10, DmarEntries::HEADER);
}

#[test]
fn dmar_errors_name_the_table_or_domain_rule_broken() {
    assert_eq!(
        DmarError::Table(AcpiError::BadChecksum).to_string(),
        "dmar is unusable: table checksum does not sum to zero"
    );
    assert_eq!(
        DmarError::ReservedRegionInverted {
            base: 0x2000,
            limit: 0x1000,
        }
        .to_string(),
        "dmar reserved region [0x2000, 0x1000] has its limit before its base"
    );
    assert_eq!(
        DmarError::DuplicateUnit {
            register_base: 0xfed9_0000
        }
        .to_string(),
        "dmar has two drhd units at register base 0xfed90000"
    );
}

#[test]
fn an_inverted_reserved_region_is_refused_by_the_validated_view() {
    let rmrr = rmrr_bytes(0, 0x2000, 0x1000, &[]);
    let bytes = table(&Sdt::DMAR, &dmar_body(&rmrr));

    assert!(DmarEntries::new(&bytes).is_ok());
    assert_eq!(
        Dmar::new(&bytes),
        Err(DmarError::ReservedRegionInverted {
            base: 0x2000,
            limit: 0x1000,
        })
    );
}

#[test]
fn a_one_byte_reserved_region_has_base_equal_to_limit_and_is_accepted() {
    let rmrr = rmrr_bytes(0, 0x3000, 0x3000, &[]);
    let bytes = table(&Sdt::DMAR, &dmar_body(&rmrr));
    assert!(Dmar::new(&bytes).is_ok());
}

#[test]
fn two_drhds_at_the_same_register_base_are_refused_by_the_validated_view() {
    let mut structures = drhd_bytes(true, 0, 0xfed9_0000, &[]);
    structures.extend_from_slice(&drhd_bytes(false, 1, 0xfed9_0000, &[]));
    let bytes = table(&Sdt::DMAR, &dmar_body(&structures));

    assert!(DmarEntries::new(&bytes).is_ok());
    assert_eq!(
        Dmar::new(&bytes),
        Err(DmarError::DuplicateUnit {
            register_base: 0xfed9_0000
        })
    );
}

#[test]
fn odd_and_empty_pci_paths_in_every_scoped_dmar_structure_are_refused() {
    for kind in [ENDPOINT, SUB_HIERARCHY] {
        for length in [6u8, 7, 9] {
            let mut malformed = vec![kind, length, 0, 0, 0, 0, 3, 1, 0xff];
            malformed.truncate(usize::from(length));
            for structure in [
                drhd_bytes(false, 0, 0x4000, &malformed),
                rmrr_bytes(0, 0x7000, 0x7fff, &malformed),
                atsr_bytes(0, &malformed),
            ] {
                let bytes = table(&Sdt::DMAR, &dmar_body(&structure));
                assert!(
                    DmarEntries::new(&bytes).is_err(),
                    "kind {kind} length {length} passed structural validation"
                );
                assert!(Dmar::new(&bytes).is_err());
            }
        }
    }
}

#[test]
fn device_scope_iteration_defensively_refuses_every_bad_pci_hop() {
    let valid = scope(ENDPOINT, 0, &[(31, 7)]);
    for malformed in [
        vec![ENDPOINT, 6, 0, 0, 0, 0],
        vec![SUB_HIERARCHY, 9, 0, 0, 0, 0, 3, 1, 0xff],
        vec![ENDPOINT, 8, 0, 0, 0, 0, 32, 0],
        vec![ENDPOINT, 8, 0, 0, 0, 0, 31, 8],
    ] {
        let joined = [malformed.as_slice(), valid.as_slice()].concat();
        let mut scopes = DeviceScopes::new(&joined);
        assert_eq!(scopes.next(), None);
        assert_eq!(
            scopes.next(),
            None,
            "iteration resumed after malformed bytes"
        );
    }

    let parsed = DeviceScopes::new(&valid)
        .next()
        .expect("maximum encodable PCI hop remains valid");
    assert_eq!(parsed.path().collect::<Vec<_>>(), [(31, 7)]);
    assert!(DeviceScopes::new(&[0x7f, 6, 0, 0, 0, 0]).next().is_some());
}

#[test]
fn a_device_scope_exposes_its_kind_start_bus_and_every_hop() {
    let scope_bytes = [1u8, 10, 0, 0, 0, 2, 3, 0, 4, 0]; // kind 1, length 10, two hops
    let drhd = drhd_bytes(false, 0, 0x4000, &scope_bytes);
    let bytes = table(&Sdt::DMAR, &dmar_body(&drhd));

    let structure = DmarEntries::new(&bytes)
        .expect("validated dmar")
        .next()
        .expect("one drhd");
    let DmarStructure::Drhd { scopes, .. } = structure else {
        unreachable!("only structure in the table is the drhd")
    };
    let scope = DeviceScopes::new(scopes).next().expect("one device scope");
    assert_eq!(scope.kind(), DeviceScopeKind::PciEndpoint);
    assert_eq!(scope.start_bus(), 2);
    assert_eq!(scope.path().collect::<Vec<_>>(), [(3, 0), (4, 0)]);
}

#[test]
fn an_unknown_device_scope_kind_is_preserved_as_other() {
    let scope_bytes = [0x7fu8, 6, 0, 0, 0, 0]; // kind 0x7f, length 6, no path
    let drhd = drhd_bytes(false, 0, 0x5000, &scope_bytes);
    let bytes = table(&Sdt::DMAR, &dmar_body(&drhd));

    let structure = DmarEntries::new(&bytes)
        .expect("validated dmar")
        .next()
        .expect("one drhd");
    let DmarStructure::Drhd { scopes, .. } = structure else {
        unreachable!("only structure in the table is the drhd")
    };
    let scope = DeviceScopes::new(scopes).next().expect("one device scope");
    assert_eq!(scope.kind(), DeviceScopeKind::Other { kind: 0x7f });
}
