use core::hint::spin_loop;

use wost::arch::current::paging::MapError;
use wost::sync::{Leaf, SpinLock};
use x86_paging::{Flags, PAGE_SIZE};

use super::address_spaces::{in_reclaimed_space, user_space};
use crate::thread::ThreadContext;
use crate::time;
use crate::timepage::{self, WorldClock};

// The test space is its own, so the page may go anywhere in the user half.
const TIME: u64 = 0x0000_1000_0000_0000;

pub(super) fn spins_until(attempts: u32, done: impl Fn() -> bool) -> bool {
    (0..attempts).any(|_| {
        spin_loop();
        done()
    })
}

pub(super) fn a_program_reads_served_time_from_a_page_it_cannot_write(
    context: &mut ThreadContext,
) -> bool {
    let clock = timepage::root();
    in_reclaimed_space(context, user_space, |space| {
        if clock.install(space, TIME).is_err() {
            return false;
        }
        let Some((frame, flags)) = space.lookup(TIME) else {
            return false;
        };

        flags.contains(Flags::USER)
            && !flags.contains(Flags::WRITABLE)
            && flags.contains(Flags::NO_EXECUTE)
            && clock.page_address() == Some(frame)
            && page_time_is_served_time(clock)
            // The frame is the kernel's, so an unmap must not hand it to the allocator.
            && matches!(
                space.unmap(TIME, PAGE_SIZE),
                Err(MapError::NotOwned(_))
            )
    })
}

// Read through the kernel's view of the frame the program maps; `Uptime` serves `now_ns`.
fn page_time_is_served_time(clock: &WorldClock) -> bool {
    const SPAN_MS: u64 = 4;

    let precision = clock.precision_ns();
    let Some(first) = clock.page_now_ns() else {
        return false;
    };
    let deadline = time::uptime_ms() + SPAN_MS;
    let mut previous = first;
    let mut agrees = true;
    while time::uptime_ms() < deadline {
        let uptime = clock.now_ns();
        let Some(page) = clock.page_now_ns() else {
            return false;
        };
        agrees &= page >= previous
            && page.is_multiple_of(precision)
            && uptime.is_multiple_of(precision)
            && page.abs_diff(uptime) <= precision;
        previous = page;
    }
    agrees && previous > first
}

pub(super) fn the_clock_never_runs_backwards() -> bool {
    const ATTEMPTS: u32 = 2_000_000;
    const SPAN_MS: u64 = 3;
    const PAIRS: u32 = 1_000;

    let deadline = time::uptime_ms() + SPAN_MS;
    let mut previous = time::now_ns();
    let mut monotonic = true;
    for attempt in 0..ATTEMPTS {
        let sample = if attempt == 0 {
            let lock = SpinLock::<Leaf, _>::new(0u32);
            let held = lock.lock_leaf();
            let sample = time::now_ns();
            drop(held);
            sample
        } else {
            time::now_ns()
        };
        monotonic &= sample >= previous;
        previous = sample;
        if sample / time::NANOSECONDS_PER_MS >= deadline {
            break;
        }
    }
    monotonic
        && (0..PAIRS).all(|_| {
            let (first, second) = time::adjacent_readings_for_test();
            second >= first
        })
}
