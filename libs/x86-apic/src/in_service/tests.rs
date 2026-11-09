use super::*;

fn vector(raw: u8) -> InterruptVector {
    InterruptVector::new(raw).expect("test vector is assignable")
}

#[test]
fn every_one_hot_is_its_exact_vector() {
    for vector in u8::MIN..=u8::MAX {
        let mut registers = [0u32; 8];
        registers[usize::from(vector) / 32] = 1 << (vector % 32);
        assert_eq!(highest(&registers), Some(vector));
    }
}

#[test]
fn the_highest_set_vector_is_the_dismissal_target() {
    let mut registers = [0u32; 8];
    for vector in [16u8, 0x40, 0x41, 0x80, 0xfe] {
        registers[usize::from(vector) / 32] |= 1 << (vector % 32);
    }
    assert_eq!(highest(&registers), Some(0xfe));
    assert_eq!(highest(&[0; 8]), None);
}

#[test]
fn absent_and_wrong_sources_plan_no_fake_eoi_write() {
    let mut registers = [0u32; 8];
    registers[2] = 1 << 1;

    for (state, requested) in [([0; 8], 0x41), (registers, 0x40)] {
        let mut writes = 0;
        if owns(&state, vector(requested)) {
            writes += 1;
        }
        assert_eq!(writes, 0, "vector {requested:#x}");
    }
    assert!(owns(&registers, vector(0x41)));
}

#[test]
fn only_named_live_ownership_observations_authorize_completion() {
    for interrupts in [Interrupts::Excluded, Interrupts::Enabled] {
        for frame_liveness in [FrameLiveness::Live, FrameLiveness::Stale] {
            for frame_vector in [0x40, 0x41] {
                for token_vector in [vector(0x40), vector(0x41)] {
                    for dismissal in [DismissalOwnership::Owned, DismissalOwnership::Unowned] {
                        let expected = interrupts == Interrupts::Excluded
                            && frame_liveness == FrameLiveness::Live
                            && frame_vector == u64::from(token_vector.get())
                            && dismissal == DismissalOwnership::Owned;
                        assert_eq!(
                            may_complete(
                                interrupts,
                                frame_liveness,
                                frame_vector,
                                token_vector,
                                dismissal,
                            ),
                            expected,
                            "{interrupts:?} {frame_liveness:?} {frame_vector:#x} \
                             {token_vector:?} {dismissal:?}"
                        );
                    }
                }
            }
        }
    }
}
