use super::*;

#[test]
fn invalid_alignments_are_refused_before_placement_arithmetic() {
    for align in [0, 3, usize::MAX] {
        assert_eq!(
            SizeClass::for_size_align(1, align),
            Err(PlacementError::InvalidAlignment(align))
        );
        assert_eq!(
            place(1, align),
            Err(PlacementError::InvalidAlignment(align))
        );
    }
}
