use super::*;

/// AVX-512 and APX at their architectural sizes, with non-compacted offsets out of index order.
fn avx512_and_apx(index: u32) -> Component {
    match index {
        2 => Component::new(256, Alignment::Natural),
        5 => Component::new(64, Alignment::Natural),
        6 => Component::new(512, Alignment::Natural),
        7 => Component::new(1024, Alignment::Natural),
        8 => Component::new(128, Alignment::Natural),
        9 => Component::new(8, Alignment::Natural),
        19 => Component::new(80, Alignment::Bytes64),
        _ => Component::ABSENT,
    }
}

/// Hand-calculated offsets, since an expectation derived from `Layout` passes a monotonic one.
#[test]
fn a_high_index_at_a_low_offset_changes_nothing() {
    let mask = 0b111 | (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 19);
    let layout = Layout::compacted(mask, avx512_and_apx);

    assert_eq!(layout.offset_of(2), Some(576), "avx");
    assert_eq!(layout.offset_of(5), Some(832), "opmask");
    assert_eq!(layout.offset_of(6), Some(896), "zmm hi256");
    assert_eq!(layout.offset_of(7), Some(1408), "hi16 zmm");
    assert_eq!(layout.offset_of(8), Some(2432), "pt");
    assert_eq!(layout.offset_of(9), Some(2560), "pkru");
    assert_eq!(layout.offset_of(0), None, "x87");
    assert_eq!(layout.offset_of(1), None, "sse");

    let apx = layout.offset_of(19).expect("apx was enabled");
    assert_eq!(apx % 64, 0, "an aligned component started unaligned");
    assert_eq!(layout.size(), apx + 80);
    assert!(layout.is_consistent());
}
