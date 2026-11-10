use alloc::sync::Arc;
use core::hint::spin_loop;
use core::sync::atomic::Ordering;

use wost::arch::current::apic;
use wost::arch::current::cpu;
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use super::address_spaces::in_reclaimed_space;
use super::runs_until;
use super::threads::{LIGHT_SPINS, STOP_SPINNING, context, light_spinner, yields_until};
use crate::arch::current::trap;
use crate::arch::current::vector;
use crate::handle::HandleTable;
use crate::queue;
use crate::timepage;
use crate::{thread, time};

pub(super) fn the_time_page_states_the_clock_the_kernel_measured() -> bool {
    let Some((version, page_bytes, counter_hz)) = timepage::constants() else {
        return false;
    };

    version == timepage::VERSION
        && page_bytes == PAGE_SIZE as u32
        // The measured rate, not merely a non-zero one: a page publishing
        // somebody else's clock would still pass a "is it plausible" test.
        && counter_hz == crate::time::counter_hz()
        && counter_hz > 0
}

pub(super) fn the_time_page_and_the_kernel_clock_tell_the_same_time() -> bool {
    let Some(reading) = timepage::read() else {
        return false;
    };
    if reading.mode != timepage::MODE_COUNTER {
        return false;
    }

    let before = crate::time::now_ns();
    let published = reading.at(cpu::timestamp());
    let after = crate::time::now_ns();

    let agrees = before <= published && published <= after;

    // And it *advances*: a page whose conversion is zero answers the same
    // number every time, which sits inside any sandwich taken quickly enough.
    let first = reading.at(cpu::timestamp());
    let moved = spins_until(1_000_000, || reading.at(cpu::timestamp()) > first);

    agrees && moved && reading.conversion.is_calibrated()
}

pub(super) fn revoking_the_clock_sends_readers_to_uptime() -> bool {
    let Some(before) = timepage::read() else {
        return false;
    };

    let deferred = timepage::with_page_absent_for_test(|| {
        timepage::publish(
            timepage::MODE_NONE,
            crate::time::origin_ticks(),
            crate::time::conversion(),
        ) == timepage::Publication::Deferred
    });
    if deferred != Some(true) {
        return false;
    }

    let Some(revoked) = timepage::read() else {
        return false;
    };
    // Exactly two, begin and end: one would leave the page permanently odd
    // and every reader spinning.
    let stepped = revoked.seq == before.seq.wrapping_add(2);
    let closed = revoked.mode == timepage::MODE_NONE;

    // The fallback, on the path a program would actually use.
    let queue_answers = {
        let Some(queue) = queue::Queue::new() else {
            return false;
        };
        queue.post_submission_for_test(
            0,
            queue::Submission::simple(queue::Opcode::Uptime, 0x7EA, 0),
        );
        queue.publish_tail_for_test(1);
        let table = shared_handle_table_or_false!(8);
        let served = queue.serve(&table) == 1;
        let completion = queue.read_completion(0);
        served
            && completion.token == 0x7EA
            && completion.status == queue::STATUS_OK
            && completion.value > 0
    };

    let restored = timepage::publish(
        timepage::MODE_COUNTER,
        crate::time::origin_ticks(),
        crate::time::conversion(),
    ) == timepage::Publication::Published
        && timepage::read().is_some_and(|reading| reading.mode == timepage::MODE_COUNTER);

    stepped && closed && queue_answers && restored
}

pub(super) fn the_clocks_update_window_is_never_preemptible() -> bool {
    // The revocation check has published twice by now, so this is not a count
    // that has never been exercised.
    let Some(seq) = timepage::sequence() else {
        return false;
    };

    timepage::is_published()
        && timepage::windows_left_preemptible() == 0
        // Read the odd marker to prove every update opened its seqlock window
        // (D-166).
        && timepage::windows_never_opened() == 0
        && timebase::Seq::is_stable(seq)
        // Publishing moves it, so "even" is not merely "never written".
        && seq >= 4
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
            // And not the space's to give away: the frame is the kernel's, so an
            // unmap must not hand it to the allocator.
            && matches!(
                space.unmap(timepage::TIME, PAGE_SIZE),
                Err(MapError::NotOwned(_))
            )
    })
}

pub(super) fn spins_until(attempts: u32, done: impl Fn() -> bool) -> bool {
    (0..attempts).any(|_| {
        spin_loop();
        done()
    })
}

pub(super) fn ping_arrives(target: usize) -> bool {
    const ATTEMPTS: u32 = 1_000_000;

    spins_until(ATTEMPTS, || trap::last_ping().count >= target)
}

pub(super) fn timer_entry_is_programmed() -> bool {
    let entry = apic::timer_entry();
    !entry.is_masked() && entry.is_one_shot() && entry.vector() == vector::TIMER.get()
}

pub(super) fn rates_are_believable() -> bool {
    (1_000_000..=10_000_000_000).contains(&time::apic_hz())
        && (500_000_000..=10_000_000_000).contains(&time::counter_hz())
}

/// What a continuous read of the clock saw.
pub(super) struct ClockSweep {
    pub(super) monotonic: bool,
    pub(super) advanced: bool,
    /// The smallest positive step between adjacent counter readings.
    pub(super) finest_step: Option<u64>,
}

pub(super) fn sweep_clock() -> ClockSweep {
    const ATTEMPTS: u32 = 2_000_000;
    const SPAN_MS: u64 = 3;
    const PAIRS: u32 = 1_000;

    let deadline = time::uptime_ms() + SPAN_MS;
    // Seed with a real reading so the first comparison, sampled under a lock,
    // is not vacuously monotonic against zero (D-124).
    let mut previous = time::now_ns();
    let mut sweep = ClockSweep {
        monotonic: true,
        advanced: false,
        finest_step: None,
    };

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

        if sample < previous {
            sweep.monotonic = false;
        }
        sweep.advanced |= sample > previous;
        previous = sample;

        if sample / time::NANOSECONDS_PER_MS >= deadline {
            break;
        }
    }
    for _ in 0..PAIRS {
        let (first, second) = time::adjacent_readings_for_test();
        match second.checked_sub(first) {
            None => sweep.monotonic = false,
            Some(0) => {}
            Some(step) => {
                sweep.finest_step = Some(sweep.finest_step.map_or(step, |finest| finest.min(step)));
            }
        }
    }
    sweep
}

pub(super) fn an_idle_kernel_takes_no_timer_interrupts() -> bool {
    // Fifty 100 Hz periods expose any surviving periodic source.
    const QUIET_MS: u64 = 500;

    let before = time::expiries();
    runs_until(QUIET_MS, || false);
    time::expiries() == before
}

pub(super) fn the_timer_fires_about_once_a_slice() -> bool {
    const WINDOW_MS: u64 = 300;
    /// `thread::SLICE_NS`, in the unit this window is measured in.
    const SLICE_MS: u64 = 3;

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("competitor", light_spinner).is_err() {
        return false;
    }

    let before = time::expiries();
    runs_until(WINDOW_MS, || false);
    let spent = time::expiries() - before;
    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);

    // A factor of four either way: the window is slice-quantised and
    // hypervisor-jittered, and the cases differ by orders of magnitude.
    let expected = WINDOW_MS / SLICE_MS;
    exited && spent >= expected / 4 && spent <= expected * 4
}

pub(super) fn a_sleeper_wakes_on_time_and_quietly() -> bool {
    /// Long enough that a surviving tick would be caught many times, short
    /// beside the harness's patience.
    const SLEEP_MS: u64 = 200;
    // Five bounded validation windows plus wake slack; a 10 ms periodic source
    // would exceed this budget fourfold.
    const EXPIRY_BUDGET: u64 = 7;
    /// How late the wake may be. Nothing else is runnable, so the thread
    /// resumes the moment its own timer fires — this is slack for the
    /// hypervisor, not for the policy.
    const LATE_MS: u64 = 50;

    let expiries = time::expiries();
    let before = time::uptime_ms();
    thread::sleep_ms(&mut context(), SLEEP_MS);
    let slept = time::uptime_ms() - before;
    let spent = time::expiries() - expiries;

    // Two-sided: armed on the APIC clock and measured on the counter's, so a
    // conversion wrong either way is a wait of the wrong length.
    (SLEEP_MS..=SLEEP_MS + LATE_MS).contains(&slept) && (1..=EXPIRY_BUDGET).contains(&spent)
}

pub(super) fn sleeping_hands_the_cpu_over() -> bool {
    const SLEEP_MS: u64 = 100;
    /// A thread with the CPU to itself counts far more than this in 100 ms;
    /// one scavenging a spin loop's leftovers counts far fewer.
    const PROGRESS: usize = 10_000;

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("while-asleep", light_spinner).is_err() {
        return false;
    }

    let woke = thread::wakes();
    thread::sleep_ms(&mut context(), SLEEP_MS);
    let progressed = LIGHT_SPINS.load(Ordering::Relaxed) >= PROGRESS;
    let woken = thread::wakes() == woke + 1;
    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);

    progressed && woken && exited
}
