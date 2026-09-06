//! The time page: what a program reads instead of calling.
//!
//! **Two regions, and the split is the point.** The constants — version, page
//! size, counter rate — have no sequence counter, because there is nothing to
//! tear: they are written once, before the page is mapped
//! anywhere, and never again. Zircon's arrangement, and it removes the most
//! error-prone reads from the protocol entirely. The clock region carries
//! Linux's seqlock, both barriers included.
//!
//! **The conversion is fixed; permission to use it is not.** Linux rewrites its
//! published clock for adjustable sources and NTP. This kernel keeps one
//! conversion, while a retained PIT validates bounded APIC/TSC windows. Two
//! consecutive conclusive disagreements freeze public time and publish
//! `MODE_NONE` through this page's sequence protocol (D-225). Suspend and NTP
//! adjustment remain unsupported rather than pretending continuity.

use core::sync::atomic::{AtomicU32, Ordering, fence};

#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicUsize;

use timebase::{Seq, Timebase};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, SharedBytes, USegment};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

/// Where the page appears in every program's space.
///
/// Fixed, and one more entry on D-46's KASLR ledger. It costs nothing while no
/// two programs share a table (D-113), and it is the address the ABI header
/// names.
pub(crate) const TIME: u64 = 0x0000_1000_0000_0000;

/// What this page's layout is called.
///
/// A reader that cannot say what shape the page is would have to guess, which
/// is IoRing's improvement over io_uring and the same argument as the ring
/// header's version word.
pub(crate) const VERSION: u32 = 1;

/// `mode`: the page is not to be used, and the caller must ask instead.
pub(crate) const MODE_NONE: u32 = 0;

/// `mode`: the counter named in the constants region, converted as published.
pub(crate) const MODE_COUNTER: u32 = 1;

/// Byte offsets of the constants region, which is the ABI.
mod constants {
    /// This layout's version.
    pub(super) const VERSION: usize = 0;
    /// Bytes in a page, so a program need not assume.
    pub(super) const PAGE_BYTES: usize = 4;
    /// The counter's measured rate, for a reader that wants to reason about
    /// resolution rather than only to read the time.
    pub(super) const COUNTER_HZ: usize = 8;
}

/// Where the clock region starts.
///
/// Its own cache line, away from the constants: the constants are read once
/// and the clock is read constantly, and sharing a line would have every read
/// of one pull the other.
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
    /// The conversion's multiplier.
    pub(super) const MULTIPLIER: usize = 32;
    /// The conversion's shift.
    pub(super) const SHIFT: usize = 40;
}

/// The one page. `None` until the global ABI page is built at boot; present
/// even when the clock is uncalibrated or revoked.
///
/// Global rather than per-process (D-152): every program reads the same clock,
/// and a page each would be a page each to keep in step.
static PAGE: SpinLock<Option<SharedBytes>> = SpinLock::new(None);

/// The mode the page must show when an installation temporarily returns it.
///
/// `install` moves the owner out while it maps the frame. A revocation may
/// interrupt that allocation window, so the desired mode cannot live only in
/// the temporarily absent page.
static DESIRED_MODE: AtomicU32 = AtomicU32::new(MODE_NONE);

/// Times the sequence counter was left odd with interrupts on.
///
/// The measured publication window, not the caller's entry state, must exclude
/// preemption so a writer cannot strand readers on an odd sequence value
/// (D-152, D-166).
#[cfg(feature = "selftest")]
static WINDOW_LEFT_PREEMPTIBLE: AtomicUsize = AtomicUsize::new(0);

/// Updates that never marked their window open.
///
/// The writer reads its own odd marker because interrupt exclusion leaves no
/// concurrent observer on one CPU; a second CPU is required for independent
/// observation (D-154, D-166).
#[cfg(feature = "selftest")]
static WINDOW_NEVER_OPENED: AtomicUsize = AtomicUsize::new(0);

/// Build the page and write its constants. Once, at boot.
///
/// The constants go in **before** anything can map it, which is what lets that
/// region have no sequence counter: a reader cannot see a half-written value
/// it could never reach.
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

/// Write the clock region, or revoke it.
///
/// `mode` is what a caller is really choosing: [`MODE_COUNTER`] publishes the
/// conversion below, and [`MODE_NONE`] tells every reader to ask instead. Both
/// go through the same sequence-counter protocol, because a revocation a
/// reader half-sees is worse than one it does not see at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a deferred publication must be observed"]
pub(crate) enum Publication {
    Published,
    Deferred,
}

pub(crate) fn publish(mode: u32, origin: u64, conversion: Timebase) -> Publication {
    DESIRED_MODE.store(mode, Ordering::Release);
    let mut guard = PAGE.lock_leaf();
    // Inside the guard, because the question is whether the window below can
    // be preempted; counted rather than refused, which the caller cannot act on.
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
    // The first of Linux's two barriers: on x86 a compiler barrier and no
    // instruction, written out so another port does not rediscover it.
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

/// Map the page into `space`, read-only.
///
/// **Not writable and not executable**, which is the whole of what a program
/// may do with it. `map_shared` adds `USER` and writes no owned bit, so a
/// program cannot free the frame and an unmap cannot return it.
///
/// # Errors
///
/// [`TimePageError::Absent`] if the page could not be built at boot, which is a
/// machine that ran out of memory before its first program; otherwise whatever
/// [`AddressSpace::map_shared`] refused.
pub(crate) fn install(space: &mut AddressSpace) -> Result<(), TimePageError> {
    // Moved out because mapping may allocate under the leaf; a publication
    // during the gap records a mode that restore applies (D-85, D-115).
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

/// Run a check while the page owner is temporarily held outside its lock.
#[cfg(feature = "selftest")]
pub(crate) fn with_page_absent_for_test<T>(check: impl FnOnce() -> T) -> Option<T> {
    let shared = PAGE.lock_leaf().take()?;
    let result = check();
    restore(shared);
    Some(result)
}

/// Why a program could not be shown the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimePageError {
    /// There is no page: the machine could not spare a frame at boot.
    ///
    /// A refusal rather than a silent omission. The ABI says the page is at
    /// [`TIME`], and a program that reads it and faults has been lied to.
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

/// Read the page the way a program does, and say what it found.
///
/// The kernel's copy of the reader protocol, and it exists so the check can be
/// about the *page* rather than about the kernel's own clock. Bounded rather
/// than looped for ever: a page whose writer never finishes is a broken kernel,
/// and spinning on it forgives exactly the failure worth reporting.
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
            seq: before,
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
#[derive(Debug, Clone, Copy)]
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
    /// The sequence counter this read agreed on.
    pub(crate) seq: u32,
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

/// Updates that never marked their window open; see [`WINDOW_NEVER_OPENED`].
#[cfg(feature = "selftest")]
pub(crate) fn windows_never_opened() -> usize {
    WINDOW_NEVER_OPENED.load(Ordering::Relaxed)
}

/// Updates whose sequence-counter window ran with interrupts on.
///
/// Zero on a correct kernel; see [`WINDOW_LEFT_PREEMPTIBLE`].
#[cfg(feature = "selftest")]
pub(crate) fn windows_left_preemptible() -> usize {
    WINDOW_LEFT_PREEMPTIBLE.load(Ordering::Relaxed)
}

/// How many times the clock region has been written.
///
/// Two per publication, so a check can say a publication happened rather than
/// inferring it from a value that might not have changed.
#[cfg(feature = "selftest")]
pub(crate) fn sequence() -> Option<u32> {
    let guard = PAGE.lock_leaf();
    Some(read_u32(guard.as_ref()?, CLOCK_AT + clock::SEQ))
}

/// Whether the page exists at all.
#[cfg(feature = "selftest")]
pub(crate) fn is_published() -> bool {
    PAGE.lock_leaf().is_some()
}

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

#[cfg(feature = "selftest")]
fn read_u64(page: &SharedBytes, at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    if !page.read_volatile_at(at, &mut bytes) {
        panic!("an internal time-page field was outside the shared page");
    }
    u64::from_le_bytes(bytes)
}
