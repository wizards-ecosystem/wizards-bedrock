use crate::bar::io_window_size;

#[test]
fn an_io_window_size_is_a_port_count() {
    for bits in 2..16u32 {
        let size = 1u32 << bits;
        let mask = !(size - 1);
        assert_eq!(
            io_window_size(mask),
            u16::try_from(size).ok(),
            "a {size}-port window"
        );
    }
    assert_eq!(io_window_size(0xFFFF_FFE1), Some(32));
    assert_eq!(io_window_size(0x0000_FFFC), Some(4));
    assert_eq!(io_window_size(0x0000_FFE0), Some(32));
}
