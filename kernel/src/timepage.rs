use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering, fence};

use native_abi::address::TIME;
use native_abi::timepage::{CLOCK_AT, VERSION, clock, constants, mode};
use timebase::{Seq, Timebase};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, SharedBytes, USegment};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
pub(crate) use selftest::read;

static PAGE: SpinLock<Option<Arc<SharedBytes>>> = SpinLock::new(None);

// A revocation that finds the page shared is written by the installation that lets go last.
static DESIRED_MODE: AtomicU32 = AtomicU32::new(mode::NONE);

pub(crate) fn init(counter_hz: u64) -> bool {
    let Some(run) = mm::allocate_contiguous(1, 1).and_then(USegment::zeroed) else {
        return false;
    };
    let mut shared = run.share().into_bytes();

    write_u32(&mut shared, constants::VERSION, VERSION);
    write_u32(&mut shared, constants::PAGE_BYTES, PAGE_SIZE as u32);
    write_u64(&mut shared, constants::COUNTER_HZ, counter_hz);

    let page = Arc::new(shared);
    *PAGE.lock_leaf() = Some(page);
    true
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[must_use = "a deferred publication must be observed"]
pub(crate) enum Publication {
    Published,
    Deferred,
}

pub(crate) fn publish(mode: u32, origin: u64, conversion: Timebase) -> Publication {
    DESIRED_MODE.store(mode, Ordering::Release);
    let mut guard = PAGE.lock_leaf();
    let Some(page) = guard.as_mut().and_then(Arc::get_mut) else {
        return Publication::Deferred;
    };
    write_clock(page, mode, origin, conversion);
    Publication::Published
}

fn write_clock(page: &mut SharedBytes, mode: u32, origin: u64, conversion: Timebase) {
    let seq = read_u32(page, CLOCK_AT + clock::SEQ);

    // Odd: every reader retries.
    write_u32(page, CLOCK_AT + clock::SEQ, Seq::begin(seq));
    // The first of Linux's two barriers, only a compiler barrier on x86.
    fence(Ordering::Release);

    write_u32(page, CLOCK_AT + clock::MODE, mode);
    write_u64(page, CLOCK_AT + clock::BASE_NS, 0);
    write_u64(page, CLOCK_AT + clock::FRACTION, 0);
    write_u64(page, CLOCK_AT + clock::LAST_TICKS, origin);
    write_u64(page, CLOCK_AT + clock::MULTIPLIER, conversion.multiplier());
    write_u32(page, CLOCK_AT + clock::SHIFT, conversion.shift());

    fence(Ordering::Release);
    // Even again: a reader keeps what was written between the two.
    write_u32(page, CLOCK_AT + clock::SEQ, Seq::end(Seq::begin(seq)));
}

// Mapped `USER` with no owned bit, so a program cannot free the frame and no unmap returns it.
pub(crate) fn install(space: &mut AddressSpace) -> Result<(), Error> {
    // Shared rather than held, because mapping may allocate, which no leaf guard may span.
    let shared = PAGE.lock_leaf().as_ref().map(Arc::clone);
    let Some(shared) = shared else {
        return Err(Error::Absent);
    };

    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE);
    drop(shared);
    apply_deferred_revocation();
    result.map_err(Error::Map)
}

// The mode is read under the guard, so a revocation either sees this share gone or is seen here.
fn apply_deferred_revocation() {
    let mut guard = PAGE.lock_leaf();
    if DESIRED_MODE.load(Ordering::Acquire) != mode::NONE {
        return;
    }
    if let Some(page) = guard.as_mut().and_then(Arc::get_mut) {
        write_clock(page, mode::NONE, 0, Timebase::UNCALIBRATED);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Error {
    Absent,
    Map(MapError),
}

impl core::fmt::Display for Error {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Absent => formatter.write_str("no time page was built"),
            Self::Map(error) => write!(formatter, "{error}"),
        }
    }
}

impl core::error::Error for Error {}

fn write_u32(page: &mut SharedBytes, at: usize, value: u32) {
    assert!(
        page.write_at(at, &value.to_le_bytes()),
        "an internal time-page field was outside the shared page"
    );
}

fn write_u64(page: &mut SharedBytes, at: usize, value: u64) {
    assert!(
        page.write_at(at, &value.to_le_bytes()),
        "an internal time-page field was outside the shared page"
    );
}

fn read_u32(page: &SharedBytes, at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    assert!(
        page.read_at(at, &mut bytes),
        "an internal time-page field was outside the shared page"
    );
    u32::from_le_bytes(bytes)
}
