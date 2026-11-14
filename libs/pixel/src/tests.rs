use super::*;

#[test]
fn a_framebuffer_address_range_that_wraps_is_refused() {
    assert_eq!(
        FramebufferLayout::new(usize::MAX - 3, 1, 1, 4, 4),
        Err(FramebufferLayoutError::AddressRangeOverflow)
    );
}
