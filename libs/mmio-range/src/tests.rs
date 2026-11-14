use super::*;

#[test]
fn a_repeated_length_that_wraps_is_refused() {
    assert_eq!(
        AccessRange::repeated(0, usize::MAX, 4, usize::MAX),
        Err(AccessError::LengthOverflow {
            count: usize::MAX,
            width: 4,
        })
    );
}
