use super::*;

#[test]
fn baseline_math_and_bands_are_two_sided() {
    let band = Band::new(10, 20);
    assert!(band.check("fixture", 10).is_ok());
    assert!(band.check("fixture", 20).is_ok());
    assert!(band.check("fixture", 9).is_err());
    assert!(band.check("fixture", 21).is_err());

    assert_eq!(
        ticks_to_units(4_096, 2_000_000_000, 400_000),
        Some(20_480_000)
    );
    assert_eq!(ticks_to_units(u64::MAX, u64::MAX, 1), None);
    assert_eq!(ticks_to_units(1, 1, 0), None);
}
