use super::*;

#[test]
fn malformed_publication_partitions_are_refused_exactly() {
    assert_eq!(SharedLayout::<0>::new(0, []), Err(SharedLayoutError::Empty));
    assert_eq!(
        SharedLayout::new(16, [usize::MAX]),
        Err(SharedLayoutError::PublicationRangeOverflow(usize::MAX))
    );
    assert_eq!(
        SharedLayout::new(16, [16]),
        Err(SharedLayoutError::PublicationOutOfBounds(16))
    );
    assert_eq!(
        SharedLayout::new(16, [2]),
        Err(SharedLayoutError::PublicationMisaligned(2))
    );
    assert_eq!(
        SharedLayout::new(16, [4, 4]),
        Err(SharedLayoutError::PublicationOverlap {
            first: 4,
            second: 4,
        })
    );
    let layout = SharedLayout::new(16, [4]).expect("one word");
    assert!(!layout.contains_payload(usize::MAX, 2));
    assert!(layout.contains_publication_word(4));
    assert!(!layout.contains_publication_word(8));
}
