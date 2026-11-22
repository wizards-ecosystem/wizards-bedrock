use core::hint::spin_loop;

use boot_record::{Exit, Fault, Record};
use native_abi::witness::DRAWN;
use native_abi::witness::behaviour::{READ_COUNTER, SERVED};
use world_tree::NodeRef;
use wost::arch::current::cpu::CounterAccess;
use wost::arch::current::paging::{MapError, UserAccess};
use wost::sync::{Leaf, SpinLock};
use x86_paging::{Flags, PAGE_SIZE};
use x86_tables::exception;

use super::address_spaces::{root_earmark, user_space};
use super::processes::last_rites_for;
use super::worlds;
use crate::thread::ThreadContext;
use crate::timepage::WorldClock;
use crate::world::{self, Time, registry};
use crate::{time, user};

// The test space is its own, so the page may go anywhere in the user half.
const TIME: u64 = 0x0000_1000_0000_0000;

// Enough of each colour for one witness at a time.
const WORLD_FRAMES: u32 = 128;

pub(super) fn spins_until(attempts: u32, done: impl Fn() -> bool) -> bool {
    (0..attempts).any(|_| {
        spin_loop();
        done()
    })
}

// Siblings with different precisions, one granted the raw time right: only the other's `rdtsc`
// faults, each program reads its own world's precision, and the two draws differ. A precision
// that is not the root's times a power of two is refused.
pub(super) fn sibling_worlds_keep_their_own_clock_and_randomness(
    context: &mut ThreadContext,
) -> bool {
    const TRAPPED: Time = Time {
        precision_ns: 2 * time::NANOSECONDS_PER_MS,
        counter: CounterAccess::Trapped,
    };
    const RAW: Time = Time {
        precision_ns: 4 * time::NANOSECONDS_PER_MS,
        counter: CounterAccess::Raw,
    };
    const OFF_LADDER: Time = Time {
        precision_ns: 3 * time::NANOSECONDS_PER_MS,
        counter: CounterAccess::Trapped,
    };

    let Some(root) = world::root() else {
        return false;
    };
    if worlds::timed(context, root, WORLD_FRAMES, OFF_LADDER).is_some() {
        return false;
    }
    let (Some(trapped), Some(raw)) = (
        worlds::timed(context, root, WORLD_FRAMES, TRAPPED),
        worlds::timed(context, root, WORLD_FRAMES, RAW),
    ) else {
        return false;
    };
    let (Some((held, _)), Some((trapped_reserved, _)), Some((raw_reserved, _))) = (
        world::inspect(root),
        world::inspect(trapped),
        world::inspect(raw),
    ) else {
        return false;
    };
    let trapped_run = served_in(context, trapped, TRAPPED.precision_ns);
    let raw_run = served_in(context, raw, RAW.precision_ns);
    let gone = worlds::killed(context, trapped, root, held + trapped_reserved)
        && worlds::killed(context, raw, root, held + trapped_reserved + raw_reserved);
    let (Some((trapped_vector, trapped_draw)), Some((raw_vector, raw_draw))) =
        (trapped_run, raw_run)
    else {
        return false;
    };
    gone && trapped_vector == u64::from(exception::GENERAL_PROTECTION)
        && raw_vector == u64::from(exception::INVALID_OPCODE)
        && trapped_draw & DRAWN != 0
        && raw_draw & DRAWN != 0
        && trapped_draw != raw_draw
}

// The vector that ended a counter read, and the draw of a program that checked its precision.
fn served_in(context: &mut ThreadContext, world: NodeRef, precision_ns: u64) -> Option<(u64, u64)> {
    let counter = user::spawn_in(context, world, None, "user-rdtsc", 12, READ_COUNTER, 0).ok()?;
    let Some(Record::Fault(Fault { vector, .. })) = last_rites_for(&counter).ending else {
        return None;
    };
    drop(counter);
    let served = user::spawn_in(
        context,
        world,
        None,
        "user-served",
        20,
        SERVED,
        precision_ns,
    )
    .ok()?;
    let Some(Record::Exit(Exit { status: draw })) = last_rites_for(&served).ending else {
        return None;
    };
    Some((vector, draw))
}

pub(super) fn a_program_reads_served_time_from_a_page_it_cannot_write(
    context: &mut ThreadContext,
) -> bool {
    let Some(clock) = world::root().and_then(registry::clock) else {
        return false;
    };
    let tables = u32::try_from(x86_paging::tables_for(1)).ok();
    let (Some(mut space), Some(page), Some(mut tables)) = (
        user_space(context),
        clock.frame(),
        tables.and_then(|frames| root_earmark(context, frames)),
    ) else {
        return false;
    };
    let mapped = space
        .map_frame(TIME, page.frame(), UserAccess::Read, &mut tables)
        .is_ok();
    let checked = mapped
        && space.lookup(TIME).is_some_and(|(frame, flags)| {
            flags.contains(Flags::USER)
                && !flags.contains(Flags::WRITABLE)
                && flags.contains(Flags::NO_EXECUTE)
                && clock.page_address() == Some(frame)
        })
        && page_time_is_served_time(clock)
        // The frame is the world's, so an unmap must not hand it to the allocator.
        && matches!(
            space.unmap(TIME, PAGE_SIZE),
            Err(MapError::NotOwned(_))
        );
    let cleared = !mapped
        || space.clear_frame(TIME, page.frame()).is_ok_and(|flush| {
            flush.wait(context);
            true
        });
    checked && cleared && space.reclaim(None).is_ok()
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
