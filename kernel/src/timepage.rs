use alloc::sync::Arc;

use native_abi::address::TIME;
use native_abi::timepage::{VERSION, constants};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, SharedBytes, USegment};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

static PAGE: SpinLock<Option<Arc<SharedBytes>>> = SpinLock::new(None);

pub(crate) fn init(counter_hz: u64) -> bool {
    let Some(run) = mm::allocate_contiguous(1, 1).and_then(USegment::zeroed) else {
        return false;
    };
    // The clock region stays zero, which reads as mode none: ring 3 has no counter to convert.
    let mut shared = run.share().into_bytes();

    write_u32(&mut shared, constants::VERSION, VERSION);
    write_u32(&mut shared, constants::PAGE_BYTES, PAGE_SIZE as u32);
    write_u64(&mut shared, constants::COUNTER_HZ, counter_hz);

    let page = Arc::new(shared);
    *PAGE.lock_leaf() = Some(page);
    true
}

// Mapped `USER` with no owned bit, so a program cannot free the frame and no unmap returns it.
pub(crate) fn install(space: &mut AddressSpace) -> Result<(), Error> {
    // Shared rather than held, because mapping may allocate, which no leaf guard may span.
    let shared = PAGE.lock_leaf().as_ref().map(Arc::clone);
    let Some(shared) = shared else {
        return Err(Error::Absent);
    };

    space
        .map_shared(TIME, &shared, Flags::NO_EXECUTE)
        .map_err(Error::Map)
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
