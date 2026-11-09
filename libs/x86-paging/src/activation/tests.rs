use super::State;

#[test]
fn every_byte_decodes_to_one_state_or_refusal() {
    for bits in u8::MIN..=u8::MAX {
        let decoded = State::from_bits(bits);
        if bits <= State::Ready.bits() {
            assert_eq!(decoded.map(State::bits), Some(bits));
        } else {
            assert_eq!(decoded, None, "unexpected state byte {bits}");
        }
    }
}

#[test]
fn only_the_protocol_edges_are_admitted() {
    assert_eq!(State::Cold.begin(), Some(State::Initializing));
    assert_eq!(State::Initializing.begin(), None);
    assert_eq!(State::Ready.begin(), None);

    assert_eq!(State::Cold.complete(), None);
    assert_eq!(State::Initializing.complete(), Some(State::Ready));
    assert_eq!(State::Ready.complete(), None);

    assert_eq!(State::Cold.abort(), None);
    assert_eq!(State::Initializing.abort(), Some(State::Cold));
    assert_eq!(State::Ready.abort(), None);
}
