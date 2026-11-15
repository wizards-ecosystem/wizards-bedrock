use super::*;

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
fn an_inner_bridge_clears_a_tie_between_the_bridges_above_it() {
    let outer = [2u8, 8, 0, 0, 0, 0, 1, 0];
    let inner = [2u8, 10, 0, 0, 0, 0, 1, 0, 2, 0];
    let units = [
        drhd_bytes(false, 0, 0x1000, &outer),
        drhd_bytes(false, 0, 0x2000, &outer),
        drhd_bytes(false, 0, 0x3000, &inner),
    ];
    let bytes = table(&Sdt::DMAR, &dmar_body(&units.concat()));
    let dmar = Dmar::new(&bytes).expect("valid");
    let bridge = |at: Bdf| match (at.bus(), at.device()) {
        (0, 1) => Some(BusRange {
            secondary: 1,
            subordinate: 4,
        }),
        (1, 2) => Some(BusRange {
            secondary: 2,
            subordinate: 3,
        }),
        _ => None,
    };
    let below = |root: Bdf, at: Bdf| at.bus() > root.bus();

    let unit = dmar.unit_for(Bdf::new(0, 2, 0, 0), bridge, below);
    assert_eq!(unit.map(|unit| unit.register_base), Some(0x3000));
}
