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
