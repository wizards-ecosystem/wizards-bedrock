use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use native_abi::address::TIME;
use native_abi::timepage::{NOW_NS, VERSION, constants, mode};
use ring_proto::SharedLayout;
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, SharedRing, USegment};
use wost::sync::{Leaf, SpinLock};
use x86_paging::{Flags, PAGE_SIZE};

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

    pub(crate) fn start(&self) -> Result<(), Error> {
        let layout = SharedLayout::new(PAGE_SIZE as usize, [NOW_NS, NOW_NS + 4])
            .map_err(|_| Error::Absent)?;
        let run = mm::allocate_contiguous(1, 1)
            .and_then(USegment::zeroed)
            .ok_or(Error::Absent)?;
        let mut page = run.share().into_ring(layout).map_err(|_| Error::Absent)?;

        let mode = if time::is_calibrated() {
            mode::NORMAL
        } else {
            mode::NONE
        };
        write(&mut page, constants::VERSION, &VERSION.to_le_bytes());
        write(
            &mut page,
            constants::PAGE_BYTES,
            &(PAGE_SIZE as u32).to_le_bytes(),
        );
        write(&mut page, constants::MODE, &mode.to_le_bytes());
        write(
            &mut page,
            constants::PRECISION_NS,
            &self.precision_ns.to_le_bytes(),
        );
        write(
            &mut page,
            constants::GENERATION,
            &FIRST_GENERATION.to_le_bytes(),
        );

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

    // Mapped `USER` with no owned bit, so a program cannot free the frame and no unmap returns it.
    pub(crate) fn install(&self, space: &mut AddressSpace) -> Result<(), Error> {
        // Shared rather than held, because mapping may allocate, which no leaf guard may span.
        let shared = self.page.lock_leaf().as_ref().map(Arc::clone);
        let Some(shared) = shared else {
            return Err(Error::Absent);
        };
        space
            .map_shared_ring(TIME, &shared, Flags::NO_EXECUTE)
            .map_err(Error::Map)
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn page_now_ns(&self) -> Option<u64> {
        self.page.lock_leaf().as_ref()?.load_acquire_u64(NOW_NS)
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn page_address(&self) -> Option<mm::PhysicalAddress> {
        Some(self.page.lock_leaf().as_ref()?.address())
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

fn write(page: &mut PageWords, at: usize, bytes: &[u8]) {
    assert!(
        page.write_payload_at(at, bytes),
        "an internal time-page field was outside the shared page"
    );
}
