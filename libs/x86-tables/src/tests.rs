use super::*;

#[test]
fn selector_packs_index_above_privilege() {
    assert_eq!(Selector::new(1, 0).bits(), 0x08);
    assert_eq!(Selector::new(2, 0).bits(), 0x10);
    assert_eq!(Selector::new(3, 0).bits(), 0x18);
    assert_eq!(Selector::new(4, 3).bits(), 0x23);
}

#[test]
fn selector_round_trips_through_its_bits() {
    let selector = Selector::new(5, 3);
    let read_back = Selector::from_bits(selector.bits());
    assert_eq!(read_back.index(), 5);
    assert_eq!(read_back.bits() & 0b11, 3);
    assert_eq!(read_back, selector);
}

#[test]
#[should_panic(expected = "privilege level is two bits")]
fn selector_rejects_an_impossible_privilege() {
    let _ = Selector::new(1, 4);
}

#[test]
#[should_panic(expected = "descriptor index does not fit a selector")]
fn an_index_past_thirteen_bits_is_refused() {
    // `index << 3` would drop the top bits and name a different
    // descriptor; nothing tested this assert (D-105).
    let _ = Selector::new(0x2000, 0);
}

#[test]
fn the_largest_index_a_selector_holds_is_accepted() {
    // Index in 15:3, table indicator clear in bit 2, RPL in 1:0.
    assert_eq!(Selector::new(0x1FFF, 3).bits(), 0xFFFB);
}
