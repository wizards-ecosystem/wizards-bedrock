use super::*;

#[test]
fn the_longest_set_1_sequence_never_says_set_2() {
    let pause = [0xE1u8, 0x1D, 0x45, 0xE1, 0x9D, 0xC5];
    let seen: Vec<Arriving> = pause.iter().map(|byte| SetProbe::observe(*byte)).collect();
    assert!(!seen.contains(&Arriving::Set2), "{seen:?}");
    assert!(seen.contains(&Arriving::Set1));
}

#[test]
fn set_2s_sysrq_make_is_not_set_1_proof() {
    assert_eq!(SetProbe::observe(0x84), Arriving::Undecided);
}
