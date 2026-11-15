use core::sync::atomic::{AtomicU64, Ordering};

use timebase::Timebase;
use wost::arch::current::cpu;

use super::{TSC_TO_NANOSECONDS, nanoseconds_at};

pub(super) static EXPIRIES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn adjacent_readings_for_test() -> (u64, u64) {
    let elapsed = Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed));
    let first = cpu::timestamp();
    let second = cpu::timestamp();
    (
        nanoseconds_at(elapsed, first),
        nanoseconds_at(elapsed, second),
    )
}

pub(crate) fn expiries() -> u64 {
    EXPIRIES.load(Ordering::Relaxed)
}
