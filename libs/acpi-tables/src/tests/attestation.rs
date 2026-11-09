use super::*;

#[test]
fn dma_coverage_attestation_is_exact_checksummed_and_typed() {
    let mut body = [0u8; 12];
    body[0..2].copy_from_slice(&7u16.to_le_bytes());
    body[4..12].copy_from_slice(&0xfed9_0000u64.to_le_bytes());
    let mut bytes = table(&Sdt::DMA_COVERAGE, &body);
    bytes[8] = 1;
    fix_checksum(&mut bytes, 9);

    let coverage = DmaCoverage::new(&bytes).expect("valid DMA coverage attestation");
    assert_eq!(coverage.segment(), 7);
    assert_eq!(coverage.register_base(), 0xfed9_0000);

    let mut wrong_revision = bytes.clone();
    wrong_revision[8] = 2;
    fix_checksum(&mut wrong_revision, 9);
    assert_eq!(
        DmaCoverage::new(&wrong_revision),
        Err(DmaCoverageError::UnsupportedRevision(2))
    );

    let mut reserved = bytes.clone();
    reserved[Sdt::LEN + 2] = 1;
    fix_checksum(&mut reserved, 9);
    assert_eq!(
        DmaCoverage::new(&reserved),
        Err(DmaCoverageError::NonzeroReserved(1))
    );

    let wrong = table(&Sdt::DMAR, &[0; 12]);
    assert!(matches!(
        DmaCoverage::new(&wrong),
        Err(DmaCoverageError::WrongTable { .. })
    ));
}

#[test]
fn dma_handoff_requires_every_exact_platform_property() {
    let mut body = [0u8; 12];
    body[0..2].copy_from_slice(&7u16.to_le_bytes());
    body[2..4].copy_from_slice(&DmaHandoff::REQUIRED_FLAGS.to_le_bytes());
    let mut bytes = table(&Sdt::DMA_HANDOFF, &body);
    bytes[8] = 1;
    fix_checksum(&mut bytes, 9);

    let handoff = DmaHandoff::new(&bytes).expect("complete DMA handoff attestation");
    assert_eq!(handoff.segment(), 7);

    for missing in [1u16, 2, 4] {
        let mut incomplete = bytes.clone();
        incomplete[Sdt::LEN + 2..Sdt::LEN + 4]
            .copy_from_slice(&(DmaHandoff::REQUIRED_FLAGS & !missing).to_le_bytes());
        fix_checksum(&mut incomplete, 9);
        assert!(matches!(
            DmaHandoff::new(&incomplete),
            Err(DmaHandoffError::UnsafeFlags(_))
        ));
    }

    let mut unknown = bytes.clone();
    unknown[Sdt::LEN + 2..Sdt::LEN + 4]
        .copy_from_slice(&(DmaHandoff::REQUIRED_FLAGS | 8).to_le_bytes());
    fix_checksum(&mut unknown, 9);
    assert_eq!(
        DmaHandoff::new(&unknown),
        Err(DmaHandoffError::UnsafeFlags(DmaHandoff::REQUIRED_FLAGS | 8))
    );

    let mut revision = bytes.clone();
    revision[8] = 2;
    fix_checksum(&mut revision, 9);
    assert_eq!(
        DmaHandoff::new(&revision),
        Err(DmaHandoffError::UnsupportedRevision(2))
    );

    let mut reserved = bytes.clone();
    reserved[Sdt::LEN + 4] = 1;
    fix_checksum(&mut reserved, 9);
    assert_eq!(
        DmaHandoff::new(&reserved),
        Err(DmaHandoffError::NonzeroReserved(1))
    );

    let mut bad_checksum = bytes.clone();
    bad_checksum[9] ^= 1;
    assert!(matches!(
        DmaHandoff::new(&bad_checksum),
        Err(DmaHandoffError::Table(AcpiError::BadChecksum))
    ));

    let wrong_length = table(&Sdt::DMA_HANDOFF, &[0; 13]);
    assert_eq!(
        DmaHandoff::new(&wrong_length),
        Err(DmaHandoffError::WrongLength(49))
    );

    let wrong_table = table(&Sdt::DMA_COVERAGE, &body);
    assert!(matches!(
        DmaHandoff::new(&wrong_table),
        Err(DmaHandoffError::WrongTable { .. })
    ));
}
