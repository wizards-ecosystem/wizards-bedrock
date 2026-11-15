use super::*;

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
