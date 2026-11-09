use super::*;

#[test]
fn admission_exhausts_all_256_vectors() {
    for raw in u8::MIN..=u8::MAX {
        assert_eq!(
            InterruptVector::new(raw).map(InterruptVector::get),
            (raw >= 0x30).then_some(raw),
            "vector {raw:#x}"
        );
    }
}

#[test]
fn every_cpu_vector_is_reserved() {
    for reserved in u8::MIN..FIRST_PIC_VECTOR {
        assert!(
            Assignment::new([0x30, reserved, 0xfe, 0xff]).is_none(),
            "reserved vector {reserved:#x}"
        );
    }
}

#[test]
fn every_remapped_pic_vector_is_reserved() {
    for reserved in FIRST_PIC_VECTOR..FIRST_ASSIGNABLE {
        assert!(
            Assignment::new([0x30, reserved, 0xfe, 0xff]).is_none(),
            "reserved vector {reserved:#x}"
        );
    }
}

#[test]
fn every_pair_is_accepted_exactly_when_assignable_and_distinct() {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            let expected = first >= 0x30 && second >= 0x30 && first != second;
            assert_eq!(
                Assignment::new([first, second]).is_some(),
                expected,
                "vectors {first:#x}, {second:#x}"
            );
        }
    }
}

#[test]
fn a_complete_assignment_preserves_source_order() {
    let assigned = Assignment::new([0xff, 0xfe, 0x40, 0x41, 0x80, 0x42])
        .expect("valid vector assignment")
        .into_array();
    assert_eq!(
        assigned.map(InterruptVector::get),
        [0xff, 0xfe, 0x40, 0x41, 0x80, 0x42]
    );
}
