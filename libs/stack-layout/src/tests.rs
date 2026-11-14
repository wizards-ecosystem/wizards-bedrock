use super::*;

#[test]
fn a_window_that_would_wrap_is_refused() {
    let top = !(PAGE_SIZE - 1);
    assert_eq!(StackLayout::new(top, 0x4000, 0x1000, 1), None);
    assert_eq!(StackLayout::new(0, !(PAGE_SIZE - 1), 0x1000, 2), None);
}
