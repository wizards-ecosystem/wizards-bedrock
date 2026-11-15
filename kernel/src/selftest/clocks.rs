use core::hint::spin_loop;

use wost::arch::current::cpu;
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use super::address_spaces::in_reclaimed_space;
use crate::{time, timepage};

pub(super) fn spins_until(attempts: u32, done: impl Fn() -> bool) -> bool {
    (0..attempts).any(|_| {
        spin_loop();
        done()
    })
}

pub(super) fn the_time_page_and_the_kernel_clock_tell_the_same_time() -> bool {
    let Some(reading) = timepage::read() else {
        return false;
    };
    if reading.mode != timepage::MODE_COUNTER {
        return false;
    }

    let before = time::now_ns();
    let published = reading.at(cpu::timestamp());
    let after = time::now_ns();
    let agrees = before <= published && published <= after;

    // A page whose conversion is zero answers one number, which falls inside any quick sandwich.
    let first = reading.at(cpu::timestamp());
    let moved = spins_until(1_000_000, || reading.at(cpu::timestamp()) > first);

    agrees && moved && reading.conversion.is_calibrated()
}

pub(super) fn a_program_sees_the_time_page_and_cannot_write_it() -> bool {
    in_reclaimed_space(AddressSpace::for_user, |space| {
        if timepage::install(space).is_err() {
            return false;
        }
        let Some((_, flags)) = space.lookup(timepage::TIME) else {
            return false;
        };

        flags.contains(Flags::USER)
            && !flags.contains(Flags::WRITABLE)
            && flags.contains(Flags::NO_EXECUTE)
            // The frame is the kernel's, so an unmap must not hand it to the allocator.
            && matches!(
                space.unmap(timepage::TIME, PAGE_SIZE),
                Err(MapError::NotOwned(_))
            )
    })
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
            let lock = SpinLock::new(0u32);
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
