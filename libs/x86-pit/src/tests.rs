use super::*;

#[test]
fn a_plain_delta_is_a_subtraction() {
    assert_eq!(elapsed_ticks(1000, 400), 600);
}

/// 100 ticks to the wrap, then another 100 down from the top.
#[test]
fn a_delta_survives_the_wrap() {
    assert_eq!(elapsed_ticks(100, 65_436), 200);
}

#[test]
fn an_unmoved_counter_is_zero_ticks() {
    assert_eq!(elapsed_ticks(5, 5), 0);
}

#[test]
fn the_longest_measurable_window_is_one_count_short_of_the_span() {
    assert_eq!(elapsed_ticks(0, 1), u16::MAX);
}

/// 8254 datasheet, read-back status: bit 7 output, bit 6 null count.
#[test]
fn the_null_count_flag_is_bit_six() {
    assert_eq!(STATUS_NULL_COUNT, 0b0100_0000);
}
