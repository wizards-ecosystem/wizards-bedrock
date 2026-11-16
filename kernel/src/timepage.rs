use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use native_abi::timepage::{Constants, NOW_NS, mode};
use ring_proto::SharedLayout;
use wost::mm::pool::ArenaFrame;
use wost::mm::{SharedFrame, SharedRing};
use wost::sync::{Leaf, SpinLock};
use x86_paging::PAGE_SIZE;

use crate::time;

pub(crate) const PRECISION_NS: u64 = time::NANOSECONDS_PER_MS;

// The refresh never runs faster than once a millisecond, however fine the precision.
const MIN_REFRESH_NS: u64 = time::NANOSECONDS_PER_MS;

const FIRST_GENERATION: u64 = 1;

// The two halves of `NOW_NS`, the only words written after the page is mapped.
type PageWords = SharedRing<2>;

#[derive(Debug)]
pub(crate) struct WorldClock {
    // Host time at which this world's time is zero, so a world never learns host uptime.
    origin_ns: AtomicU64,
    precision_ns: u64,
    served_ns: AtomicU64,
    page: SpinLock<Leaf, Option<Arc<PageWords>>>,
}

static ROOT: WorldClock = WorldClock::new(PRECISION_NS);

pub(crate) fn root() -> &'static WorldClock {
    &ROOT
}

impl WorldClock {
    const fn new(precision_ns: u64) -> Self {
        Self {
            origin_ns: AtomicU64::new(0),
            precision_ns,
            served_ns: AtomicU64::new(0),
            page: SpinLock::new(None),
        }
    }

    // The page is a frame of the world it serves.
    pub(crate) fn start(&self, frame: ArenaFrame) -> Result<(), Error> {
        let layout = SharedLayout::new(PAGE_SIZE as usize, [NOW_NS, NOW_NS + 4])
            .map_err(|_| Error::Absent)?;
        let mut page = frame.share().into_ring(layout).map_err(|_| Error::Absent)?;

        let mode = if time::is_calibrated() {
            mode::NORMAL
        } else {
            mode::NONE
        };
        let constants = Constants {
            page_bytes: PAGE_SIZE as u32,
            mode,
            precision_ns: self.precision_ns,
            generation: FIRST_GENERATION,
        };
        write(&mut page, 0, &constants.encode());

        self.origin_ns.store(time::now_ns(), Ordering::Release);
        *self.page.lock_leaf() = Some(Arc::new(page));
        Ok(())
    }

    // Rounded down to the precision and raised by a maximum, so no reader sees time go back.
    pub(crate) fn now_ns(&self) -> u64 {
        let world = self.to_world_ns(time::now_ns());
        let rounded = world - world % self.precision_ns;
        let page = self.page.lock_leaf();
        let Some(page) = page.as_ref() else {
            return 0;
        };
        let served = self
            .served_ns
            .fetch_max(rounded, Ordering::AcqRel)
            .max(rounded);
        assert!(
            page.raise_u64(NOW_NS, served).is_some(),
            "the time page lost its now word"
        );
        served
    }

    // A deadline `after_ns` past the served time, rounded up so it is itself a served value.
    pub(crate) fn deadline_after(&self, after_ns: u64) -> u64 {
        let wanted = self.now_ns().saturating_add(after_ns);
        wanted
            .div_ceil(self.precision_ns)
            .saturating_mul(self.precision_ns)
    }

    pub(crate) fn to_host_ns(&self, world_ns: u64) -> u64 {
        world_ns.saturating_add(self.origin_ns.load(Ordering::Acquire))
    }

    pub(crate) fn to_world_ns(&self, host_ns: u64) -> u64 {
        host_ns.saturating_sub(self.origin_ns.load(Ordering::Acquire))
    }

    pub(crate) fn refresh_interval_ns(&self) -> u64 {
        self.precision_ns.max(MIN_REFRESH_NS)
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn precision_ns(&self) -> u64 {
        self.precision_ns
    }

    // A space keeps its own reference, so the page outlives every leaf naming it.
    pub(crate) fn frame(&self) -> Option<SharedFrame> {
        self.page.lock_leaf().as_ref().map(|page| page.frame())
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn page_now_ns(&self) -> Option<u64> {
        self.page.lock_leaf().as_ref()?.load_acquire_u64(NOW_NS)
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn page_address(&self) -> Option<wost::mm::PhysicalAddress> {
        Some(self.page.lock_leaf().as_ref()?.address())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Error {
    Absent,
}

impl core::fmt::Display for Error {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Absent => formatter.write_str("no time page was built"),
        }
    }
}

impl core::error::Error for Error {}

fn write(page: &mut PageWords, at: usize, bytes: &[u8]) {
    assert!(
        page.write_payload_at(at, bytes),
        "an internal time-page field was outside the shared page"
    );
}
