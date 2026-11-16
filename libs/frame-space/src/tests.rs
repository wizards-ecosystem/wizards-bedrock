use super::*;

type Small = FrameSpace<1>;

const FRAMES: usize = 64;

const SMALL_BYTES: u64 = FRAMES as u64 * FRAME_SIZE;

fn drain<const WORDS: usize>(space: &mut FrameSpace<WORDS>) -> usize {
    for taken in 0..=(64 * WORDS) {
        if space.allocate().is_none() {
            return taken;
        }
    }
    panic!("allocation did not stop after the complete frame domain");
}

#[test]
fn a_free_run_straddling_the_hint_is_found() {
    let mut space = Small::new();
    space.insert(0, SMALL_BYTES);
    assert_eq!(drain(&mut space), FRAMES);
    for frame in 30..34 {
        space.free(frame).expect("allocated");
    }
    assert_eq!(space.allocate_contiguous(2, 1), Some(30));
    space.free(30).expect("allocated");
    space.free(31).expect("allocated");

    assert_eq!(space.allocate_contiguous(4, 1), Some(30));
}

#[test]
fn insert_of_a_vast_range_returns_promptly() {
    let mut space = Small::new();
    let inserted = space.insert(0, u64::MAX - 1);
    assert_eq!(space.free_count(), FRAMES);
    assert_eq!(
        inserted.ignored,
        ((u64::MAX - 1) / FRAME_SIZE) as usize - FRAMES
    );
}

#[test]
fn a_reserved_frame_survives_a_drain_and_an_arena_keeps_to_its_colours() {
    let colours = Colours::new(4).expect("a power of two");
    let mask = |bits| ColourMask::new(colours, bits).expect("a mask within the colours");
    let amount = |colour, count| {
        let mut amounts = ColourCounts::ZERO;
        assert!(amounts.set(colour, count), "colour {colour} is in range");
        amounts
    };
    let mut space = Small::new();
    space.insert(0, SMALL_BYTES);
    let mut pool = Pool::new();
    let mut first = Reservation::new(mask(0b0010));
    let mut second = Reservation::new(mask(0b0110));

    assert!(space.reserve(colours, &mut pool, &mut first, &amount(1, 10)));
    assert!(
        !space.reserve(colours, &mut pool, &mut second, &amount(1, 7)),
        "only six frames of colour one are unreserved"
    );
    assert!(space.reserve(colours, &mut pool, &mut second, &amount(1, 6)));

    let full = ColourMask::full(colours);
    while space.allocate_unreserved(colours, &pool, full).is_some() {}
    assert_eq!(space.free_in(colours, 1), 16);
    assert_eq!(space.free_count(), 16);

    for _ in 0..10 {
        let frame = space.allocate_reserved(colours, &mut pool, &mut first);
        assert_eq!(frame.map(|frame| colours.of(frame)), Some(1));
    }
    assert_eq!(
        space.allocate_reserved(colours, &mut pool, &mut first),
        None,
        "an arena past its reservation is refused"
    );
    for _ in 0..6 {
        assert!(
            space
                .allocate_reserved(colours, &mut pool, &mut second)
                .is_some()
        );
    }
    assert_eq!(space.free_count(), 0);
}
