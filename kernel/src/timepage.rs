//! The time page a program reads instead of calling: constants written once before it is mapped
//! anywhere, which need no sequence counter, and a clock region under Linux's seqlock.
//!
//! The conversion is fixed. Two consecutive watchdog disagreements freeze public time and publish
//! `MODE_NONE` through the same sequence protocol; suspend and NTP adjustment are unsupported.

use core::sync::atomic::{AtomicU32, Ordering, fence};

use timebase::{Seq, Timebase};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, SharedBytes, USegment};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
use selftest::{WINDOW_LEFT_PREEMPTIBLE, WINDOW_NEVER_OPENED};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    constants, is_published, read, sequence, windows_left_preemptible, windows_never_opened,
    with_page_absent_for_test,
};

/// Where the page appears in every program's space, fixed because the ABI header names it.
///
/// A fixed address costs KASLR nothing only while no two programs share a table.
pub(crate) const TIME: u64 = 0x0000_1000_0000_0000;

/// This page's layout version, so a reader need not guess its shape.
pub(crate) const VERSION: u32 = 1;

/// `mode`: the page is not to be used, and the caller must ask instead.
pub(crate) const MODE_NONE: u32 = 0;

/// `mode`: the counter named in the constants region, converted as published.
pub(crate) const MODE_COUNTER: u32 = 1;

/// Byte offsets of the constants region, which is the ABI.
mod constants {
    pub(super) const VERSION: usize = 0;
    /// Bytes in a page, so a program need not assume.
    pub(super) const PAGE_BYTES: usize = 4;
    /// The counter's measured rate, for a reader reasoning about resolution.
    pub(super) const COUNTER_HZ: usize = 8;
}

/// The clock region's own cache line: it is read constantly and the constants once, so they share
/// none.
const CLOCK_AT: usize = 64;

/// Byte offsets within the clock region, which is also the ABI.
mod clock {
    /// The sequence counter. Odd means a write is in progress.
    pub(super) const SEQ: usize = 0;
    /// Whether this page may be used at all.
    pub(super) const MODE: usize = 4;
    /// Whole nanoseconds at the moment `LAST_TICKS` was read.
    pub(super) const BASE_NS: usize = 8;
    /// The fraction of a nanosecond carried past `BASE_NS`.
    pub(super) const FRACTION: usize = 16;
    /// The counter reading `BASE_NS` belongs to.
    pub(super) const LAST_TICKS: usize = 24;
    pub(super) const MULTIPLIER: usize = 32;
    pub(super) const SHIFT: usize = 40;
}

/// The one global page: `None` until built at boot, and present even when the clock is
/// uncalibrated or revoked. Every program reads the same clock, so one page keeps them in step.
static PAGE: SpinLock<Option<SharedBytes>> = SpinLock::new(None);

/// The mode the page must show when an installation returns it. A revocation may interrupt the
/// window while `install` has the owner moved out, so the mode cannot live only in the page.
static DESIRED_MODE: AtomicU32 = AtomicU32::new(MODE_NONE);

/// Build the page and write its constants, once at boot, before anything can map it.
pub(crate) fn init(counter_hz: u64) -> bool {
    let Some(mut run) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !run.zero() {
        return false;
    }
    let mut shared = USegment::from(run).share().into_bytes();

    write_u32(&mut shared, constants::VERSION, VERSION);
    write_u32(&mut shared, constants::PAGE_BYTES, PAGE_SIZE as u32);
    write_u64(&mut shared, constants::COUNTER_HZ, counter_hz);

    *PAGE.lock_leaf() = Some(shared);
    true
}

/// Whether [`publish`] wrote the page or found it absent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[must_use = "a deferred publication must be observed"]
pub(crate) enum Publication {
    Published,
    Deferred,
}

/// Write the clock region: [`MODE_COUNTER`] publishes the conversion, and [`MODE_NONE`] revokes it.
///
/// Both go through the sequence protocol, because a revocation a reader half-sees is worse than one
/// it does not see at all.
pub(crate) fn publish(mode: u32, origin: u64, conversion: Timebase) -> Publication {
    DESIRED_MODE.store(mode, Ordering::Release);
    let mut guard = PAGE.lock_leaf();
    // Inside the guard, because the question is whether the window can be preempted; counted,
    // since the caller cannot act on a refusal.
    #[cfg(feature = "selftest")]
    if wost::arch::current::interrupt::are_enabled() {
        WINDOW_LEFT_PREEMPTIBLE.fetch_add(1, Ordering::Relaxed);
    }

    let Some(page) = guard.as_mut() else {
        return Publication::Deferred;
    };

    let seq = read_u32(page, CLOCK_AT + clock::SEQ);

    // Odd, and every reader now knows to try again.
    write_u32(page, CLOCK_AT + clock::SEQ, Seq::begin(seq));
    #[cfg(feature = "selftest")]
    if Seq::is_stable(read_u32(page, CLOCK_AT + clock::SEQ)) {
        WINDOW_NEVER_OPENED.fetch_add(1, Ordering::Relaxed);
    }
    // The first of Linux's two barriers, only a compiler barrier on x86, written out for any port.
    fence(Ordering::Release);

    write_u32(page, CLOCK_AT + clock::MODE, mode);
    write_u64(page, CLOCK_AT + clock::BASE_NS, 0);
    write_u64(page, CLOCK_AT + clock::FRACTION, 0);
    write_u64(page, CLOCK_AT + clock::LAST_TICKS, origin);
    write_u64(page, CLOCK_AT + clock::MULTIPLIER, conversion.multiplier());
    write_u32(page, CLOCK_AT + clock::SHIFT, conversion.shift());

    fence(Ordering::Release);
    // Even again, and what was written between the two is what a reader keeps.
    write_u32(page, CLOCK_AT + clock::SEQ, Seq::end(Seq::begin(seq)));
    Publication::Published
}

/// Map the page into `space`, never writable or executable.
///
/// `map_shared` adds `USER` and writes no owned bit, so a program cannot free the frame and an
/// unmap cannot return it.
///
/// # Errors
///
/// [`TimePageError::Absent`] if the page could not be built at boot, otherwise whatever
/// [`AddressSpace::map_shared`] refused.
pub(crate) fn install(space: &mut AddressSpace) -> Result<(), TimePageError> {
    // Moved out because mapping may allocate, which no leaf guard may span; a publication during
    // the gap records a mode that `restore` applies.
    let taken = PAGE.lock_leaf().take();
    let Some(shared) = taken else {
        return Err(TimePageError::Absent);
    };

    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE);
    restore(shared);
    result.map_err(TimePageError::Map)
}

fn restore(shared: SharedBytes) {
    *PAGE.lock_leaf() = Some(shared);
    if DESIRED_MODE.load(Ordering::Acquire) == MODE_NONE {
        let publication = publish(MODE_NONE, 0, Timebase::UNCALIBRATED);
        if publication != Publication::Published {
            panic!("a restored time page refused its deferred publication");
        }
    }
}

/// Why a program could not be shown the time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TimePageError {
    /// No frame could be spared at boot; refused, since the ABI promises the page at [`TIME`].
    Absent,
    /// The mapping itself was refused.
    Map(MapError),
}

impl core::fmt::Display for TimePageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Absent => core::fmt::Write::write_str(f, "no time page was built"),
            Self::Map(error) => write!(f, "{error}"),
        }
    }
}

impl core::error::Error for TimePageError {}

fn write_u32(page: &mut SharedBytes, at: usize, value: u32) {
    assert!(
        page.write_volatile_at(at, &value.to_le_bytes()),
        "an internal time-page field was outside the shared page"
    );
}

fn write_u64(page: &mut SharedBytes, at: usize, value: u64) {
    assert!(
        page.write_volatile_at(at, &value.to_le_bytes()),
        "an internal time-page field was outside the shared page"
    );
}

fn read_u32(page: &SharedBytes, at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    if !page.read_volatile_at(at, &mut bytes) {
        panic!("an internal time-page field was outside the shared page");
    }
    u32::from_le_bytes(bytes)
}
