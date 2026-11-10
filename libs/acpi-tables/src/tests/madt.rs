use super::*;

#[test]
fn interrupt_errors_name_the_table_or_inventory_bound() {
    assert_eq!(
        InterruptError::Table(AcpiError::BadChecksum).to_string(),
        "madt is unusable: table checksum does not sum to zero"
    );
    assert_eq!(
        InterruptError::TooManyIoApics { limit: 16 }.to_string(),
        "madt describes more than 16 i/o apics"
    );
}

// Minima from ACPI 6.5 table 5-22, independent of the parser.
#[test]
fn a_madt_entry_shorter_than_its_fields_is_refused_not_read() {
    for (kind, minimum) in [(0u8, 8usize), (1, 12), (2, 10), (0x80, 2)] {
        let short = minimum - 1;
        let mut madt = madt_with_entry(kind, short);
        assert!(
            MadtEntries::new(&madt).is_err(),
            "kind {kind} accepted {short} bytes, one short of {minimum}"
        );

        madt = madt_with_entry(kind, minimum);
        let entries = MadtEntries::new(&madt).expect("a long-enough entry");
        assert_eq!(entries.count(), 1);
    }
}

fn madt_with_entry(kind: u8, len: usize) -> Vec<u8> {
    const MADT_HEADER: usize = Sdt::LEN + 8;

    let mut table = vec![0u8; MADT_HEADER + len];
    table[..4].copy_from_slice(&Sdt::MADT);
    let length = u32::try_from(table.len()).expect("fits");
    table[4..8].copy_from_slice(&length.to_le_bytes());
    table[8] = 1;
    if let Some(slot) = table.get_mut(MADT_HEADER) {
        *slot = kind;
    }
    if let Some(slot) = table.get_mut(MADT_HEADER + 1) {
        *slot = u8::try_from(len).expect("a byte");
    }
    let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    table[9] = table[9].wrapping_sub(checksum);
    table
}

#[test]
fn a_madt_yields_the_entries_this_kernel_acts_on() {
    let mut body = Vec::new();
    body.extend_from_slice(&0xfee0_0000u32.to_le_bytes()); // local APIC
    body.extend_from_slice(&1u32.to_le_bytes()); // flags
    body.extend_from_slice(&[0, 8, 3, 7, 1, 0, 0, 0]); // local APIC entry
    body.extend_from_slice(&[1, 12, 2, 0]); // I/O APIC entry
    body.extend_from_slice(&0xfec0_0000u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&[2, 10, 0, 1]); // source override, IRQ 1
    body.extend_from_slice(&9u32.to_le_bytes());
    body.extend_from_slice(&0x000fu16.to_le_bytes());
    body.extend_from_slice(&[4, 6, 0, 0, 0, 0]); // an NMI entry, not ours

    let bytes = table(&Sdt::MADT, &body);
    let entries: Vec<_> = MadtEntries::new(&bytes).expect("validated madt").collect();
    assert_eq!(
        entries,
        [
            MadtEntry::LocalApic {
                processor: 3,
                id: 7,
                enabled: true
            },
            MadtEntry::IoApic {
                id: 2,
                address: 0xfec0_0000,
                gsi_base: 0
            },
            MadtEntry::InterruptSourceOverride {
                bus: 0,
                source: 1,
                gsi: 9,
                flags: 0x000f
            },
            MadtEntry::Other { kind: 4 },
        ]
    );
}

#[test]
fn a_local_apic_without_the_enabled_bit_is_disabled() {
    let bytes = madt_with_entry(0, 8);
    assert_eq!(
        core::iter::Iterator::next(&mut MadtEntries::new(&bytes).expect("valid madt")),
        Some(MadtEntry::LocalApic {
            processor: 0,
            id: 0,
            enabled: false,
        })
    );
}

#[test]
fn io_apic_accessors_return_each_firmware_field() {
    let controller = IoApic {
        id: 7,
        address: 0xfec1_0000,
        gsi_base: 24,
    };
    assert_eq!(controller.id(), 7);
    assert_eq!(controller.address(), 0xfec1_0000);
    assert_eq!(controller.gsi_base(), 24);
}

fn madt_with_overrides(overrides: &[(u8, u8, u32, u16)]) -> Vec<u8> {
    let mut body = vec![0u8; 8];
    for (bus, source, gsi, flags) in overrides {
        body.extend_from_slice(&[2, 10, *bus, *source]);
        body.extend_from_slice(&gsi.to_le_bytes());
        body.extend_from_slice(&flags.to_le_bytes());
    }
    table(&Sdt::MADT, &body)
}

#[test]
fn interrupt_overrides_are_unique_isa_sources_with_defined_flags() {
    for entries in [[(0, 1, 9, 0), (0, 1, 10, 0)], [(0, 1, 10, 0), (0, 1, 9, 0)]] {
        assert_eq!(
            MadtInterrupts::<1>::parse(&madt_with_overrides(&entries)),
            Err(InterruptError::DuplicateOverride(1))
        );
    }

    assert_eq!(
        MadtInterrupts::<1>::parse(&madt_with_overrides(&[(1, 1, 9, 0)])),
        Err(InterruptError::NonIsaOverride { bus: 1, source: 1 })
    );
    assert_eq!(
        MadtInterrupts::<1>::parse(&madt_with_overrides(&[(0, 16, 9, 0)])),
        Err(InterruptError::InvalidOverrideSource(16))
    );
    for flags in [0b0010, 0b1000, 0x0010] {
        assert_eq!(
            MadtInterrupts::<1>::parse(&madt_with_overrides(&[(0, 1, 9, flags)])),
            Err(InterruptError::ReservedOverrideFlags { source: 1, flags })
        );
    }
}

#[test]
fn a_bounded_interrupt_inventory_keeps_every_controller() {
    let mut body = vec![0u8; 8];
    body.extend_from_slice(&[1, 12, 7, 0]);
    body.extend_from_slice(&0xfec0_0000u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&[2, 10, 0, 1]);
    body.extend_from_slice(&25u32.to_le_bytes());
    body.extend_from_slice(&0x000fu16.to_le_bytes());
    body.extend_from_slice(&[1, 12, 9, 0]);
    body.extend_from_slice(&0xfec1_0000u32.to_le_bytes());
    body.extend_from_slice(&24u32.to_le_bytes());
    let bytes = table(&Sdt::MADT, &body);

    let inventory = MadtInterrupts::<2>::parse(&bytes).expect("two controllers fit");
    assert_eq!(inventory.io_apic_count(), 2);
    assert_eq!(
        inventory.io_apics().collect::<Vec<_>>(),
        [
            IoApic {
                id: 7,
                address: 0xfec0_0000,
                gsi_base: 0,
            },
            IoApic {
                id: 9,
                address: 0xfec1_0000,
                gsi_base: 24,
            },
        ]
    );
    assert_eq!(inventory.gsi_for(1), 25);
    assert_eq!(inventory.flags_for(1), 0x000f);
    assert_eq!(inventory.gsi_for(2), 2);
    assert_eq!(inventory.flags_for(2), 0);
}

#[test]
fn an_interrupt_inventory_refuses_overflow_instead_of_truncating() {
    let mut body = vec![0u8; 8];
    for (id, address, base) in [
        (1, 0xfec0_0000u32, 0u32),
        (2, 0xfec1_0000, 24),
        (3, 0xfec2_0000, 48),
    ] {
        body.extend_from_slice(&[1, 12, id, 0]);
        body.extend_from_slice(&address.to_le_bytes());
        body.extend_from_slice(&base.to_le_bytes());
    }
    let bytes = table(&Sdt::MADT, &body);

    assert_eq!(
        MadtInterrupts::<2>::parse(&bytes).map(|inventory| inventory.io_apic_count()),
        Err(InterruptError::TooManyIoApics { limit: 2 })
    );
}

// The corrupt input that would otherwise loop forever.
#[test]
fn a_madt_entry_claiming_zero_length_invalidates_the_table() {
    let mut body = vec![0u8; 8];
    body.extend_from_slice(&[1, 0, 0, 0]);
    let bytes = table(&Sdt::MADT, &body);
    assert!(matches!(
        MadtEntries::new(&bytes),
        Err(AcpiError::MalformedEntry { length: 0, .. })
    ));
}

#[test]
fn a_madt_without_its_fixed_header_is_invalid() {
    let bytes = table(&Sdt::MADT, &[]);

    assert_eq!(
        MadtEntries::new(&bytes).map(|entries| entries.count()),
        Err(AcpiError::BadLength(Sdt::LEN as u32))
    );
}

#[test]
fn a_corrupt_tail_invalidates_valid_madt_entries_before_it() {
    let mut body = vec![0u8; 8];
    body.extend_from_slice(&[1, 12, 2, 0]);
    body.extend_from_slice(&0xfec0_0000u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&[0, 0]);
    let bytes = table(&Sdt::MADT, &body);

    assert!(matches!(
        MadtEntries::new(&bytes),
        Err(AcpiError::MalformedEntry { length: 0, .. })
    ));
}

#[test]
fn a_madt_entry_running_past_the_table_invalidates_it() {
    let mut body = vec![0u8; 8];
    body.extend_from_slice(&[1, 40, 0, 0]);
    let bytes = table(&Sdt::MADT, &body);
    assert!(matches!(
        MadtEntries::new(&bytes),
        Err(AcpiError::MalformedEntry {
            length: 40,
            available: 4,
            ..
        })
    ));
}

#[test]
fn a_truncated_entry_of_a_known_kind_invalidates_the_table() {
    let mut body = vec![0u8; 8];
    body.extend_from_slice(&[1, 10, 2, 0, 0, 0, 0, 0, 0, 0]);
    let bytes = table(&Sdt::MADT, &body);
    assert!(matches!(
        MadtEntries::new(&bytes),
        Err(AcpiError::MalformedEntry { length: 10, .. })
    ));
}
