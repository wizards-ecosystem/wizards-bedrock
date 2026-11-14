use super::*;

#[test]
fn a_monotonic_poll_limit_expires_by_time_not_iteration_rate() {
    let mut limit = PollLimit::after_ms(Some(1_000_000_000), 500, 1);
    for _ in 0..10_000 {
        assert!(limit.permit(Some(1_499_999_999)));
    }
    assert!(!limit.permit(Some(1_500_000_000)));
    assert!(!limit.permit(None), "a vanished clock fails closed");
}

#[test]
fn a_deadline_saturates_instead_of_wrapping_into_the_past() {
    let mut limit = PollLimit::after_ms(Some(u64::MAX - 1), u64::MAX, 0);
    assert!(limit.permit(Some(u64::MAX - 1)));
    assert!(!limit.permit(Some(u64::MAX)));
}
