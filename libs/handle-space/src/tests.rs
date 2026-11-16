use crate::{Entry, Mixer, Slot};

#[test]
fn a_slot_retires_and_a_value_decodes_only_under_its_own_mixer() {
    let last = (1u64 << 44) - 1;
    let mut entry = Entry::Free(last - 2);
    let mut issued = [0u64; 3];
    for room in &mut issued {
        let (live, generation) = entry.fill().expect("a free entry fills");
        *room = generation;
        entry = live.empty(generation).expect("a live entry empties");
    }
    assert_eq!(issued, [last - 2, last - 1, last]);
    assert_eq!(entry, Entry::Retired);
    assert_eq!(entry.fill(), None);
    assert_eq!(Entry::Live(last).empty(last - 1), None);

    let mine = Mixer::from_derived(0x0123_4567_89AB_CDEFu64.to_le_bytes());
    let theirs = Mixer::from_derived(0xFEDC_BA98_7654_3210u64.to_le_bytes());
    let slot = Slot::new(5, 7).expect("a small slot is valid");
    let value = slot.encode(mine);
    assert_ne!(value, 0);
    assert_eq!(Slot::decode(value, mine), Some(slot));
    assert_ne!(Slot::decode(value, theirs), Some(slot));
    assert_ne!(slot.encode(theirs), value);
}
