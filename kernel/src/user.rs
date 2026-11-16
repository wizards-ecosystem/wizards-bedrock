use native_abi::address::{RING, STACK};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::arch::current::task;
use wost::mm::PhysicalAddress;
use wost::sync::{Leaf, SpinLock};
use x86_paging::PAGE_SIZE;

use crate::handle::HandleTable;
use crate::{queue, thread};

mod call;
mod construction;
mod load;
mod reap;
#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
pub(crate) use call::refusals;
pub(crate) use call::{call, report_execution, report_refusals};
pub(crate) use construction::{SpawnError, spawn};
use load::MAX_SEGMENTS;
pub(crate) use load::keep;
#[cfg(feature = "selftest")]
pub(crate) use load::{LoadError, image, load_image};
pub(crate) use reap::{
    CollectOutcome, collect, has_zombies, orphan, record_ending, zombie_slots_for_spawn,
};
#[cfg(feature = "selftest")]
pub(crate) use reap::{LastRites, last_rites_of};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    spawn_duplicating, spawn_executing_syscall, spawn_faulting, spawn_for_bootstrap,
    spawn_into_the_copy_window, spawn_probing, spawn_reading_the_counter, spawn_reading_the_kernel,
    spawn_setting_its_thread_base, spawn_waiting_on_the_ring, spawn_with_invalid_return_flags,
    spawn_with_invalid_return_stack,
};

#[cfg(feature = "selftest")]
pub(crate) const CODE: u64 = 0x0000_4000_0000_0000;

// Keeps two programs' addresses distinct, though their page tables already isolate them.
const APART: u64 = 0x1_0000;

const STACK_BYTES: u64 = PAGE_SIZE;

#[derive(Clone, Copy, Debug)]
struct Launch {
    entry: u64,
    stack: u64,
    argument: u64,
    operand: u64,
    bootstrap_handle: u32,
    which: u64,
    root: PhysicalAddress,
}

static LAUNCHES: SpinLock<Leaf, [Option<Launch>; thread::CAPACITY]> =
    SpinLock::new([None; thread::CAPACITY]);

const MAX_REGIONS: usize = MAX_SEGMENTS + 1;

#[derive(Debug)]
struct Regions {
    ranges: [(u64, u64); MAX_REGIONS],
    len: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct UnmapRefusal {
    pages: usize,
    reason: MapError,
}

impl Regions {
    const fn new() -> Self {
        Self {
            ranges: [(0, 0); MAX_REGIONS],
            len: 0,
        }
    }

    #[must_use = "a full ledger recorded nothing"]
    fn record(&mut self, at: u64, len: u64) -> bool {
        let Some(slot) = self.ranges.get_mut(self.len) else {
            return false;
        };
        *slot = (at, len);
        self.len += 1;
        true
    }

    // Last mapped first; a refused range and every one below it stay recorded for a retry.
    fn unmap(&mut self, space: &mut AddressSpace) -> Result<usize, UnmapRefusal> {
        let mut pages = 0usize;
        while self.len != 0 {
            let (at, len) = self.ranges[self.len - 1];
            let returned = space
                .unmap(at, len)
                .map_err(|reason| UnmapRefusal { pages, reason })?;
            pages = pages.saturating_add(returned.pages() as usize);
            drop(returned);
            self.len -= 1;
        }
        Ok(pages)
    }
}

#[derive(Debug)]
struct Bootstrap {
    space: AddressSpace,
    authority: Option<BootstrapAuthority>,
    regions: Regions,
    table: alloc::sync::Arc<HandleTable>,
    ring: alloc::sync::Arc<queue::Queue>,
}

#[derive(Debug)]
enum BootstrapAuthority {
    Bound(task::Bound),
    Released(task::Released),
}

static BOOTSTRAPS: SpinLock<Leaf, [Option<Bootstrap>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

#[derive(Debug)]
pub(crate) struct Spawned {
    pub(crate) slot: usize,
    #[cfg(feature = "selftest")]
    pub(crate) observed_table_incarnation: u64,
}

pub(crate) fn run() -> Result<Spawned, SpawnError> {
    spawn("user", 0, BEHAVE, 0)
}

const BEHAVE: u64 = 0;
