//! The time page: what a program reads instead of calling.
//!
//! D-152 settled the shape and D-149 puts it in this phase, before a
//! filesystem exists to want it, because a call that becomes a load later is
//! two doors to one operation for ever. `sys_clock_get` was OWED over
//! `libs/timebase` (D-114); it is answered here by not being a call.
//!
//! **Two regions, and the split is the point.** The constants — version, page
//! size, counter rate — have no sequence counter and no version, because there
//! is nothing to tear: they are written once, before the page is mapped
//! anywhere, and never again. Zircon's arrangement, and it removes the most
//! error-prone reads from the protocol entirely. The clock region carries
//! Linux's seqlock, both barriers included.
//!
//! **This clock does not need a periodic update, and that is not a shortcut.**
//! Linux rewrites its published clock at every tick because its clocksource may
//! be an HPET, or a TSC that stops in idle, and because NTP adjusts the
//! multiplier continuously. This kernel refuses to calibrate at all unless the
//! counter is invariant (`time::init`), so the conversion is fixed and the
//! origin never moves: a reader computes `now = convert(rdtsc - origin)`, which
//! is *exactly* what [`time::now_ns`](crate::time::now_ns) computes. The
//! seqlock is still here and still correct, because the page is republished on
//! the two transitions that do exist — calibration, and revocation.
//!
//! **The revocation gate is why the syscall stays.** `mode` is the reader's
//! permission to use the page at all. A kernel that learns its counter is not
//! to be trusted sets it to zero, and every reader falls back to asking. D-152
//! requires that fallback to be always reachable, which means the ring's
//! `Uptime` opcode is not decoration.

use core::sync::atomic::{AtomicUsize, Ordering, fence};

use timebase::{Seq, Timebase};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, Shared, USegment};
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

/// The one page. `None` until the clock is measured.
///
/// Global rather than per-process (D-152): every program reads the same clock,
/// and a page each would be a page each to keep in step.
static PAGE: SpinLock<Option<Shared>> = SpinLock::new(None);

/// Times the sequence counter was left odd with interrupts on.
///
/// D-152 makes "the writer is not preemptible" a **precondition, not a
/// comment**: Linux tolerates a naive reader spin only because its update runs
/// with interrupts off, and a writer preempted mid-update leaves every reader
/// spinning for a scheduling quantum.
///
/// What is measured is the **window**, not the call. A kernel thread may
/// perfectly well call [`publish`] with interrupts on — the self-test does —
/// and nothing is wrong with that, because the guard clears the flag before
/// the counter goes odd and restores it after it goes even again. Counting the
/// call instead measured a property that was never the requirement, and the
/// first check written against it failed for being true (D-166).
///
/// So this should never move, and it can: a `SpinLock` that stopped clearing
/// the flag, or a second publisher that wrote the region without one, would be
/// caught the first time it ran.
static WINDOW_LEFT_PREEMPTIBLE: AtomicUsize = AtomicUsize::new(0);

/// Updates that never marked their window open.
///
/// The odd value is what tells a concurrent reader to try again, and on one
/// CPU **nobody can see it**: the guard clears the interrupt flag, so no other
/// context runs between the two increments, and a writer that skipped the odd
/// write entirely left every check on this page green (D-166).
///
/// It is one of the properties that genuinely wants a second CPU, and the
/// answer here is the one D-154 takes elsewhere: build the seam now and check
/// what can be checked. The writer reads its own marker back and counts a
/// failure, so a writer that stops announcing itself is caught by the only
/// observer that exists.
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
    let mut shared: Shared = USegment::from(run).share();

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
pub(crate) fn publish(mode: u32, origin: u64, conversion: Timebase) -> bool {
    let mut guard = PAGE.lock_leaf();
    // Inside the guard, which is the whole point: this asks whether the
    // *window* below can be preempted, and the guard is what closes it. Not a
    // refusal — a stale page for a reason the caller cannot act on is worse
    // than a counted surprise.
    if wost::arch::current::interrupt::are_enabled() {
        WINDOW_LEFT_PREEMPTIBLE.fetch_add(1, Ordering::Relaxed);
    }

    let Some(page) = guard.as_mut() else {
        return false;
    };

    let mut header = [0u8; 4];
    if !page.read_at(CLOCK_AT + clock::SEQ, &mut header) {
        return false;
    }
    let seq = u32::from_le_bytes(header);

    // Odd, and every reader now knows to try again.
    write_u32(page, CLOCK_AT + clock::SEQ, Seq::begin(seq));
    // Read straight back, because on one CPU this is the only witness there
    // will ever be. See `WINDOW_NEVER_OPENED`.
    if Seq::is_stable(read_u32(page, CLOCK_AT + clock::SEQ)) {
        WINDOW_NEVER_OPENED.fetch_add(1, Ordering::Relaxed);
    }
    // The first of Linux's two barriers. On x86 stores do not pass stores, so
    // this is a compiler barrier and no instruction — which is exactly what
    // `smp_wmb()` is there. Writing it out means the aarch64 port does not
    // have to rediscover that it is not free everywhere.
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
    true
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
    // Taken out of the lock and put back, rather than mapped under the guard.
    // `map_shared` allocates page-table frames, and holding a leaf guard
    // across an allocation is what `lock_leaf` forbids (D-85). Copying the
    // address out instead would hand on a bare number where the type is the
    // evidence (D-115), so the run itself moves.
    //
    // The window is real and benign: a publication landing while the page is
    // out finds `None` and does nothing. The kernel's own clock is unaffected —
    // `now_ns` reads the counter directly — so a skipped publication leaves the
    // page holding a conversion that is still true.
    let taken = PAGE.lock_leaf().take();
    let Some(shared) = taken else {
        return Err(TimePageError::Absent);
    };

    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE);
    *PAGE.lock_leaf() = Some(shared);
    result.map_err(TimePageError::Map)
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
            Self::Absent => f.write_str("no time page was built"),
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
            ),
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

fn write_u32(page: &mut Shared, at: usize, value: u32) {
    page.write_at(at, &value.to_le_bytes());
}

fn write_u64(page: &mut Shared, at: usize, value: u64) {
    page.write_at(at, &value.to_le_bytes());
}

fn read_u32(page: &Shared, at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    page.read_at(at, &mut bytes);
    u32::from_le_bytes(bytes)
}

#[cfg(feature = "selftest")]
fn read_u64(page: &Shared, at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    page.read_at(at, &mut bytes);
    u64::from_le_bytes(bytes)
}
