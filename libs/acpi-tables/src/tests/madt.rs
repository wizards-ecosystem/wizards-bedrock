use super::*;

#[test]
fn a_madt_entry_claiming_zero_length_invalidates_the_table() {
    let mut body = vec![0u8; 8];
    body.extend_from_slice(&[1, 0, 0, 0]);
    let bytes = table(Sdt::MADT, &body);
    assert!(matches!(
        MadtEntries::new(&bytes),
        Err(AcpiError::MalformedEntry { length: 0, .. })
    ));
}

fn local_apic(uid: u8, apic_id: u8, flags: u32) -> Vec<u8> {
    let mut entry = vec![0, 8, uid, apic_id];
    entry.extend_from_slice(&flags.to_le_bytes());
    entry
}

fn local_x2apic(apic_id: u32, uid: u32, flags: u32) -> Vec<u8> {
    let mut entry = vec![9, 16, 0, 0];
    entry.extend_from_slice(&apic_id.to_le_bytes());
    entry.extend_from_slice(&flags.to_le_bytes());
    entry.extend_from_slice(&uid.to_le_bytes());
    entry
}

fn local_apic_nmi(uid: u8, flags: u16, lint: u8) -> Vec<u8> {
    let mut entry = vec![4, 6, uid];
    entry.extend_from_slice(&flags.to_le_bytes());
    entry.push(lint);
    entry
}

fn local_x2apic_nmi(uid: u32, flags: u16, lint: u8) -> Vec<u8> {
    let mut entry = vec![0xa, 12];
    entry.extend_from_slice(&flags.to_le_bytes());
    entry.extend_from_slice(&uid.to_le_bytes());
    entry.extend_from_slice(&[lint, 0, 0, 0]);
    entry
}

fn processors(entries: &[Vec<u8>]) -> Result<MadtProcessors<3>, ProcessorError> {
    let mut body = vec![0u8; 8];
    for entry in entries {
        body.extend_from_slice(entry);
    }
    MadtProcessors::parse(&table(Sdt::MADT, &body))
}

#[test]
fn a_malformed_processor_list_refuses_the_table() {
    let model = processors(&[
        local_apic_nmi(0xff, 0b0101, 1),
        local_apic(4, 0, 1),
        local_apic(5, 0xff, 0),
        local_x2apic(300, 6, 2),
        local_x2apic(301, 7, 0),
    ])
    .unwrap();
    assert_eq!(model.count(), 2, "disabled processors are dropped");
    let second = model.processor(1).unwrap();
    assert_eq!(
        (second.apic_id(), second.uid(), second.state()),
        (300, 6, ProcessorState::OnlineCapable),
        "the x2apic entry is kept"
    );
    assert_eq!(
        second.nmi().map(|nmi| (nmi.lint(), nmi.flags())),
        Some((1, 0b0101)),
        "the all-processor nmi reaches every processor"
    );

    let mut short = local_x2apic(1, 1, 1);
    short[1] = 12;
    short.truncate(12);
    let cases = [
        (vec![short], "a short x2apic entry"),
        (
            vec![local_apic(0, 1, 1), local_x2apic(1, 2, 1)],
            "a repeated apic id",
        ),
        (
            vec![local_apic(3, 1, 1), local_x2apic(2, 3, 1)],
            "a repeated uid",
        ),
        (vec![local_apic(0, 1, 1 | 4)], "a reserved processor flag"),
        (vec![local_apic(0, 1, 3)], "enabled and online-capable"),
        (vec![local_x2apic(u32::MAX, 1, 1)], "the broadcast id"),
        (
            vec![
                local_apic(0, 0, 1),
                local_apic(1, 1, 1),
                local_apic(2, 2, 1),
                local_apic(3, 3, 2),
            ],
            "a fourth processor",
        ),
        (
            vec![local_apic(0, 1, 1), local_apic_nmi(0, 0, 2)],
            "a third interrupt line",
        ),
        (
            vec![local_apic(0, 1, 1), local_x2apic_nmi(0, 0b10, 1)],
            "a reserved nmi polarity",
        ),
        (
            vec![
                local_apic(0, 1, 1),
                local_apic_nmi(0xff, 0, 1),
                local_x2apic_nmi(0, 0, 0),
            ],
            "a second nmi",
        ),
    ];
    let expected = [
        ProcessorError::Table(AcpiError::MalformedEntry {
            offset: Sdt::LEN + 8,
            length: 12,
            available: 12,
        }),
        ProcessorError::DuplicateApicId(1),
        ProcessorError::DuplicateUid(3),
        ProcessorError::ReservedProcessorFlags {
            apic_id: 1,
            flags: 5,
        },
        ProcessorError::ReservedProcessorFlags {
            apic_id: 1,
            flags: 3,
        },
        ProcessorError::BroadcastApicId(u32::MAX),
        ProcessorError::TooManyProcessors { limit: 3 },
        ProcessorError::InvalidNmiLine { lint: 2 },
        ProcessorError::ReservedNmiFlags(0b10),
        ProcessorError::DuplicateNmi { apic_id: 1 },
    ];
    for ((entries, case), error) in cases.iter().zip(expected) {
        assert_eq!(processors(entries), Err(error), "{case}");
    }
}
