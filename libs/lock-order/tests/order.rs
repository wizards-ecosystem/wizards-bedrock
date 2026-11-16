use lock_order::{HeldRanks, Key, LEAF, Level, LockBefore, Pair, Unlocked, impl_lock_after, pair};

enum Outer {}
enum Middle {}
enum Inner {}

impl Level for Outer {
    const RANK: u8 = 1;
}

impl Level for Middle {
    const RANK: u8 = 40;
}

impl Level for Inner {
    const RANK: u8 = LEAF;
}

impl_lock_after!(Unlocked => Outer);
impl_lock_after!(Outer => Middle);
impl_lock_after!(Middle => Inner);

fn ordered<Held: LockBefore<Taken>, Taken>() {}

#[test]
fn a_lock_is_taken_only_above_every_rank_held_and_equal_ranks_pair_in_key_order() {
    ordered::<Unlocked, Outer>();
    ordered::<Unlocked, Inner>();
    ordered::<Outer, Inner>();
    ordered::<Middle, Inner>();

    let ranks = [1u8, 2, 40, LEAF];
    for subset in 0u32..16 {
        let mut held = HeldRanks::empty();
        let mut highest = 0u8;
        for (position, rank) in ranks.into_iter().enumerate() {
            if subset & (1 << position) != 0 {
                held = HeldRanks::from_bits(held.bits() | 1 << rank);
                highest = rank;
            }
        }
        for rank in 0..=u8::MAX {
            let admitted = rank != 0 && rank <= LEAF && rank > highest;
            let taken = held.take(rank);
            assert_eq!(
                taken.is_some(),
                admitted,
                "rank {rank} over held word {:#x}",
                held.bits()
            );
            if let Some(taken) = taken {
                assert_eq!(
                    taken.release(rank),
                    Some(held),
                    "releasing rank {rank} restores the word"
                );
            }
        }
    }

    let keys = [
        Key::new(0, 0),
        Key::new(0, 7),
        Key::new(3, 1),
        Key::new(3, 9),
    ];
    for (first_position, first) in keys.into_iter().enumerate() {
        for (second_position, second) in keys.into_iter().enumerate() {
            let expected = if first_position < second_position {
                Some(Pair::AsGiven)
            } else if first_position > second_position {
                Some(Pair::Swapped)
            } else {
                None
            };
            assert_eq!(pair(first, second), expected, "{first:?} with {second:?}");
        }
    }
}
