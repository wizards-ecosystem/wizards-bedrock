use super::*;

#[test]
fn a_conversion_is_exact_and_survives_publication() {
    let elapsed = Timebase::nanoseconds(2_500_000_000).unwrap();
    assert_eq!(elapsed.convert(2_500_000_000), 1_000_000_000);

    let countdown = Timebase::ticks(1_000_000_000).unwrap();
    assert_eq!(countdown.convert(3_000_000), 3_000_000);

    assert_eq!(Timebase::from_bits(elapsed.to_bits()), elapsed);
}
