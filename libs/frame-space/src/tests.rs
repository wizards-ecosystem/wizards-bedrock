use super::*;

#[test]
fn frame_identity_is_exact_at_zero_high_and_misaligned_boundaries() {
    let cases = [
        (0, 1),
        (0x1000, 2),
        (0x2000, 3),
        (0x0000_1000_0000_0000, 0x0000_0001_0000_0001),
        (0xffff_ffff_ffff_e000, 0x000f_ffff_ffff_ffff),
        (0xffff_ffff_ffff_f000, 0x0010_0000_0000_0000),
    ];
    let identities = cases.map(|(address, expected)| {
        let identity = frame_identity(address).expect("an aligned frame has an identity");
        assert_eq!(identity.get(), expected, "frame {address:#x}");
        identity
    });
    for (index, identity) in identities.iter().enumerate() {
        assert!(
            !identities[..index].contains(identity),
            "distinct physical frames share one identity"
        );
    }
    for base in [0, 0x1000, 0xffff_ffff_ffff_f000] {
        for offset in 1..0x1000 {
            let address = base + offset;
            assert_eq!(
                frame_identity(address),
                None,
                "unaligned address {address:#x}"
            );
        }
    }
}

/// Sixty-four frames, small enough that a test can hold several on the stack.
type Small = FrameSpace<1>;

/// Two words, so a run or a scan can cross the boundary between them.
type TwoWord = FrameSpace<2>;

fn fully_managed() -> Small {
    let mut space = Small::new();
    space.insert(0, Small::CEILING);
    space
}

fn drain<const WORDS: usize>(space: &mut FrameSpace<WORDS>) -> usize {
    for taken in 0..=FrameSpace::<WORDS>::FRAME_COUNT {
        if space.allocate().is_none() {
            return taken;
        }
    }
    panic!("allocation did not stop after the complete frame domain");
}

#[test]
fn words_for_converts_bytes_to_bitmap_words() {
    assert_eq!(words_for(FRAME_SIZE * 64), 1);
    assert_eq!(words_for(16 << 30), 65_536);
    assert_eq!(words_for(FRAME_SIZE * 65), 2);
    assert_eq!(words_for(FRAME_SIZE * 64 + 1), 2);
    assert_eq!(Small::FRAME_COUNT, 64);
    assert_eq!(Small::CEILING, FRAME_SIZE * 64);
}

#[test]
fn a_new_space_manages_nothing() {
    let space = Small::new();
    assert_eq!(space.free_count(), 0);
    assert_eq!(space.managed_count(), 0);
    assert!(!space.is_managed(0));
    assert!(!space.is_free(0));
}

#[test]
fn membership_stops_at_the_bitmap_ceiling() {
    let space = fully_managed();
    assert!(space.is_managed(Small::FRAME_COUNT - 1));
    assert!(space.is_free(Small::FRAME_COUNT - 1));
    assert!(!space.is_managed(Small::FRAME_COUNT));
    assert!(!space.is_free(Small::FRAME_COUNT));
    assert!(!space.is_managed(usize::MAX));
    assert!(!space.is_free(usize::MAX));
}

#[test]
fn allocation_count_distinguishes_the_complete_domain_from_too_many() {
    let mut exact = fully_managed();
    assert_eq!(exact.allocate_contiguous(Small::FRAME_COUNT, 1), Some(0));

    let mut oversized = fully_managed();
    assert_eq!(
        oversized.allocate_contiguous(Small::FRAME_COUNT + 1, 1),
        None
    );
    assert_eq!(oversized.allocate_contiguous(usize::MAX, 1), None);
}

#[test]
fn release_errors_name_the_frame_and_ownership_failure() {
    let foreign = FreeError::NotManaged(17).to_string();
    assert!(
        foreign.contains("17") && foreign.contains("not managed"),
        "{foreign}"
    );
    let duplicate = FreeError::AlreadyFree(23).to_string();
    assert!(
        duplicate.contains("23") && duplicate.contains("already free"),
        "{duplicate}"
    );
}

#[test]
fn debug_reports_both_ledger_counts() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 3);
    assert_eq!(space.allocate(), Some(0));
    let report = format!("{space:?}");
    assert!(report.contains("FrameSpace"), "{report}");
    assert!(report.contains("managed: 3"), "{report}");
    assert!(report.contains("free: 2"), "{report}");
}

#[test]
fn insert_takes_only_whole_frames_inside_the_range() {
    let mut space = Small::new();
    let inserted = space.insert(FRAME_SIZE + 1, FRAME_SIZE * 3);
    assert_eq!(inserted.added, 2);
    assert!(!space.is_managed(1));
    assert!(space.is_managed(2));
    assert!(space.is_managed(3));
    assert!(!space.is_managed(4));
}

#[test]
fn insert_is_idempotent_across_overlapping_regions() {
    let mut space = Small::new();
    assert_eq!(space.insert(0, FRAME_SIZE * 4).added, 4);
    assert_eq!(space.insert(0, FRAME_SIZE * 4).added, 0);
    assert_eq!(space.managed_count(), 4);
    assert_eq!(space.free_count(), 4);
}

#[test]
fn insert_reports_frames_above_the_ceiling() {
    let mut space = Small::new();
    let inserted = space.insert(Small::CEILING - FRAME_SIZE, FRAME_SIZE * 4);
    assert_eq!(inserted.added, 1, "only the last frame under the ceiling");
    assert_eq!(inserted.ignored, 3);
    assert_eq!(space.managed_count(), 1);
}

#[test]
fn insert_survives_a_range_that_would_overflow() {
    let mut space = Small::new();
    assert_eq!(space.insert(u64::MAX - 1, 8), Inserted::default());
}

#[test]
fn allocation_hands_out_managed_frames_and_free_returns_them() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 4);

    let first = space.allocate().expect("four frames are free");
    assert!(space.is_managed(first));
    assert!(!space.is_free(first));
    assert_eq!(space.free_count(), 3);

    assert_eq!(space.free(first), Ok(()));
    assert!(space.is_free(first));
    assert_eq!(space.free_count(), 4);
}

#[test]
fn allocation_stops_when_the_space_is_empty() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 2);
    assert!(space.allocate().is_some());
    assert!(space.allocate().is_some());
    assert_eq!(space.allocate(), None);
    assert_eq!(space.free_count(), 0);
}

#[test]
fn a_double_free_and_a_foreign_frame_are_both_refused() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 2);
    let frame = space.allocate().expect("a frame is free");

    assert_eq!(space.free(frame), Ok(()));
    assert_eq!(space.free(frame), Err(FreeError::AlreadyFree(frame)));
    assert_eq!(space.free(40), Err(FreeError::NotManaged(40)));
    assert_eq!(
        space.free(Small::FRAME_COUNT + 1),
        Err(FreeError::NotManaged(Small::FRAME_COUNT + 1)),
        "a frame past the ceiling is not ours either"
    );
    assert_eq!(space.free_count(), 2, "a refusal changes nothing");
}

#[test]
fn a_contiguous_run_skips_past_a_hole() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 2);
    space.insert(FRAME_SIZE * 3, FRAME_SIZE * 13);

    let run = space
        .allocate_contiguous(4, 1)
        .expect("frames 3..7 are a clear run");
    assert_eq!(run, 3, "the run before the hole is too short");
    for frame in run..run + 4 {
        assert!(!space.is_free(frame));
    }
}

#[test]
fn search_treats_its_upper_frame_as_exclusive() {
    let mut space = Small::new();
    space.insert(FRAME_SIZE * 8, FRAME_SIZE * 4);

    assert_eq!(space.search(0, 8, 4, 1), None);
    assert_eq!(space.search(0, 12, 4, 1), Some(8));
}

#[test]
fn a_contiguous_run_honours_alignment() {
    let mut space = TwoWord::new();
    space.insert(FRAME_SIZE, FRAME_SIZE * 63);

    let run = space
        .allocate_contiguous(8, 8)
        .expect("frame 8 starts an aligned run");
    assert_eq!(run, 8);
    assert!(run.is_multiple_of(8));
}

#[test]
fn a_run_may_cross_the_boundary_between_two_bitmap_words() {
    let mut space = TwoWord::new();
    space.insert(FRAME_SIZE * 60, TwoWord::CEILING - FRAME_SIZE * 60);

    let run = space.allocate_contiguous(8, 1).expect("60..68 is free");
    assert_eq!(run, 60);
    assert!(!space.is_free(63) && !space.is_free(64), "both words moved");
    assert_eq!(space.free_contiguous(run, 8), Ok(()));
    assert!(space.is_free(63) && space.is_free(64));
}

#[test]
fn alignment_is_rounded_up_to_a_power_of_two() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 64);
    let run = space
        .allocate_contiguous(1, 3)
        .expect("a frame aligned to four exists");
    assert!(run.is_multiple_of(4), "3 rounds up to 4, not down to 2");
}

#[test]
fn a_run_longer_than_the_space_is_refused() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 4);
    assert_eq!(space.allocate_contiguous(5, 1), None);
    assert_eq!(space.free_count(), 4, "a refused run takes nothing");
    assert_eq!(
        space.allocate_contiguous(0, 1),
        None,
        "zero frames is not a run"
    );
}

#[test]
fn an_impossible_run_count_is_refused_without_wrapping() {
    let mut space = Small::new();
    space.insert(0, Small::CEILING);
    assert_eq!(space.allocate(), Some(0), "leave a non-zero search hint");

    assert_eq!(space.allocate_contiguous(usize::MAX, 1), None);
    assert_eq!(space.free_count(), Small::FRAME_COUNT - 1);
}

#[test]
fn an_alignment_that_cannot_round_to_a_power_of_two_is_refused() {
    let mut space = Small::new();
    space.insert(0, Small::CEILING);
    assert_eq!(space.allocate_contiguous(1, usize::MAX), None);
    assert_eq!(space.free_count(), Small::FRAME_COUNT);
}

#[test]
fn a_run_that_leaves_the_space_reports_the_ceiling_before_any_frame() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 8);
    let run = space.allocate_contiguous(4, 1).expect("eight are free");
    space.free(run + 1).expect("one frame inside the run");

    assert_eq!(
        space.free_contiguous(run, Small::FRAME_COUNT + 1),
        Err(FreeError::NotManaged(Small::FRAME_COUNT))
    );
    assert!(!space.is_free(run), "a refused release changes nothing");
}

#[test]
fn a_release_range_that_wraps_is_refused() {
    let mut space = Small::new();
    space.insert(FRAME_SIZE, FRAME_SIZE);
    let frame = space.allocate().expect("one frame is free");
    assert_eq!(frame, 1);

    assert_eq!(
        space.free_contiguous(frame, usize::MAX),
        Err(FreeError::NotManaged(Small::FRAME_COUNT))
    );
    assert!(!space.is_free(frame), "a refused release changes nothing");
}

#[test]
fn freeing_a_run_is_all_or_nothing() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 8);
    let run = space.allocate_contiguous(4, 1).expect("eight are free");

    assert_eq!(
        space.free_contiguous(run, 5),
        Err(FreeError::AlreadyFree(run + 4))
    );
    assert!(
        !space.is_free(run),
        "a refused release leaves the run allocated"
    );

    assert_eq!(space.free_contiguous(run, 4), Ok(()));
    assert_eq!(space.free_count(), 8);
}

#[test]
fn the_hint_wraps_so_freed_frames_are_found_again() {
    let mut space = Small::new();
    space.insert(0, FRAME_SIZE * 4);
    let frames: Vec<_> = (0..4).map(|_| space.allocate().expect("free")).collect();
    assert_eq!(space.allocate(), None);

    space.free(frames[0]).expect("allocated");
    assert_eq!(space.allocate(), Some(frames[0]));
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

#[test]
fn a_sweep_of_the_whole_space_conserves_every_frame() {
    let mut space = TwoWord::new();
    let inserted = space.insert(0, TwoWord::CEILING);
    assert_eq!(inserted.added, TwoWord::FRAME_COUNT);
    assert_eq!(inserted.ignored, 0);
    assert_eq!(space.free_count(), TwoWord::FRAME_COUNT);

    let taken = drain(&mut space);
    assert_eq!(taken, TwoWord::FRAME_COUNT);
    assert_eq!(space.free_count(), 0);

    for frame in 0..TwoWord::FRAME_COUNT {
        assert_eq!(space.free(frame), Ok(()));
    }
    assert_eq!(space.free_count(), TwoWord::FRAME_COUNT);
    assert_eq!(space.managed_count(), TwoWord::FRAME_COUNT);
}
