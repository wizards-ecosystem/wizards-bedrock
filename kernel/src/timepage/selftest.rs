use core::sync::atomic::{Ordering, fence};

use timebase::{Seq, Timebase};
use wost::mm::SharedBytes;

use super::constants::{self};
use super::{CLOCK_AT, PAGE, clock, read_u32};

/// Read the page the way a program does, and say what it found.
///
/// Bounded, because a writer that never finishes is a broken kernel, and spinning on it would
/// forgive exactly the failure worth reporting.
#[cfg(feature = "selftest")]
pub(crate) fn read() -> Option<Reading> {
    const ATTEMPTS: usize = 64;

    for _ in 0..ATTEMPTS {
        let guard = PAGE.lock_leaf();
        let page = guard.as_ref()?;

        let before = read_u32(page, CLOCK_AT + clock::SEQ);
        if !Seq::is_stable(before) {
            continue;
        }
        fence(Ordering::Acquire);

        let reading = Reading {
            mode: read_u32(page, CLOCK_AT + clock::MODE),
            base_ns: read_u64(page, CLOCK_AT + clock::BASE_NS),
            fraction: read_u64(page, CLOCK_AT + clock::FRACTION),
            last_ticks: read_u64(page, CLOCK_AT + clock::LAST_TICKS),
            conversion: Timebase::from_parts(
                read_u64(page, CLOCK_AT + clock::MULTIPLIER),
                read_u32(page, CLOCK_AT + clock::SHIFT),
            )?,
        };

        fence(Ordering::Acquire);
        let after = read_u32(page, CLOCK_AT + clock::SEQ);
        if Seq::agrees(before, after) {
            return Some(reading);
        }
    }
    None
}

/// What one read of the clock region found.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reading {
    /// Whether the page may be used at all.
    pub(crate) mode: u32,
    /// Whole nanoseconds at `last_ticks`.
    pub(crate) base_ns: u64,
    /// The fraction carried past `base_ns`.
    pub(crate) fraction: u64,
    /// The counter reading the base belongs to.
    pub(crate) last_ticks: u64,
    /// How counter ticks become nanoseconds.
    pub(crate) conversion: Timebase,
}

#[cfg(feature = "selftest")]
impl Reading {
    /// What this page says the time is, given a counter reading.
    pub(crate) fn at(self, ticks: u64) -> u64 {
        self.conversion.resume(
            self.base_ns,
            self.fraction,
            ticks.wrapping_sub(self.last_ticks),
        )
    }
}

/// The constants region, which needs no protocol to read.
#[cfg(feature = "selftest")]
pub(crate) fn constants() -> Option<(u32, u32, u64)> {
    let guard = PAGE.lock_leaf();
    let page = guard.as_ref()?;
    Some((
        read_u32(page, constants::VERSION),
        read_u32(page, constants::PAGE_BYTES),
        read_u64(page, constants::COUNTER_HZ),
    ))
}

#[cfg(feature = "selftest")]
fn read_u64(page: &SharedBytes, at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    if !page.read_volatile_at(at, &mut bytes) {
        panic!("an internal time-page field was outside the shared page");
    }
    u64::from_le_bytes(bytes)
}
