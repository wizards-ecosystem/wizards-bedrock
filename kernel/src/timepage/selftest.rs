use core::sync::atomic::{Ordering, fence};

use timebase::{Seq, Timebase};
use wost::mm::SharedBytes;

use super::{CLOCK_AT, PAGE, clock, read_u32};

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

#[derive(Clone, Copy, Debug)]
pub(crate) struct Reading {
    pub(crate) mode: u32,
    pub(crate) base_ns: u64,
    pub(crate) fraction: u64,
    pub(crate) last_ticks: u64,
    pub(crate) conversion: Timebase,
}

impl Reading {
    pub(crate) fn at(self, ticks: u64) -> u64 {
        self.conversion.resume(
            self.base_ns,
            self.fraction,
            ticks.wrapping_sub(self.last_ticks),
        )
    }
}

fn read_u64(page: &SharedBytes, at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    assert!(
        page.read_at(at, &mut bytes),
        "an internal time-page field was outside the shared page"
    );
    u64::from_le_bytes(bytes)
}
