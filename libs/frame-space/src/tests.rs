use super::*;

/// Sixty-four frames, small enough that a test can hold several on the stack.
type Small = FrameSpace<1>;

fn drain<const WORDS: usize>(space: &mut FrameSpace<WORDS>) -> usize {
    for taken in 0..=FrameSpace::<WORDS>::FRAME_COUNT {
        if space.allocate().is_none() {
            return taken;
        }
    }
    panic!("allocation did not stop after the complete frame domain");
}

#[test]
fn a_free_run_straddling_the_hint_is_found() {
    let mut space = Small::new();
    space.insert(0, Small::CEILING);
    assert_eq!(drain(&mut space), Small::FRAME_COUNT);
    for frame in 30..34 {
        space.free(frame).expect("allocated");
    }
    assert_eq!(space.allocate_contiguous(2, 1), Some(30));
    space.free(30).expect("allocated");
    space.free(31).expect("allocated");

    // The hint sits at 32, inside the free run 30..34.
    assert_eq!(space.allocate_contiguous(4, 1), Some(30));
}

#[test]
fn insert_of_a_vast_range_returns_promptly() {
    let mut space = Small::new();
    let inserted = space.insert(0, u64::MAX - 1);
    assert_eq!(inserted.added, Small::FRAME_COUNT);
    assert_eq!(
        inserted.ignored,
        ((u64::MAX - 1) / FRAME_SIZE) as usize - Small::FRAME_COUNT
    );
}
