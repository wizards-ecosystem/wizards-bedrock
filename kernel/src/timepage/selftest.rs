use core::sync::atomic::{AtomicUsize, Ordering, fence};

use timebase::{Seq, Timebase};
use wost::mm::SharedBytes;

use super::constants::{self};
use super::{CLOCK_AT, PAGE, clock, read_u32, restore};

/// Times the sequence counter was left odd with interrupts on.
///
/// The measured publication window, not the caller's entry state, must exclude
/// preemption so a writer cannot strand readers on an odd sequence value
/// (D-152, D-166).
#[cfg(feature = "selftest")]
pub(super) static WINDOW_LEFT_PREEMPTIBLE: AtomicUsize = AtomicUsize::new(0);

/// Updates that never marked their window open.
///
/// The writer reads its own odd marker because interrupt exclusion leaves no
/// concurrent observer on one CPU; a second CPU is required for independent
/// observation (D-154, D-166).
#[cfg(feature = "selftest")]
pub(super) static WINDOW_NEVER_OPENED: AtomicUsize = AtomicUsize::new(0);

/// Run a check while the page owner is temporarily held outside its lock.
#[cfg(feature = "selftest")]
pub(crate) fn with_page_absent_for_test<T>(check: impl FnOnce() -> T) -> Option<T> {
    let shared = PAGE.lock_leaf().take()?;
    let result = check();
    restore(shared);
    Some(result)
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

#[cfg(feature = "selftest")]
fn read_u64(page: &SharedBytes, at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    if !page.read_volatile_at(at, &mut bytes) {
        panic!("an internal time-page field was outside the shared page");
    }
    u64::from_le_bytes(bytes)
}
