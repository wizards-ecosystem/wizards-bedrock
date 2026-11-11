//! The first thing that runs in ring 3, proving that the descriptors, the gate, the privilege
//! stack and the return path all agree before a loader makes disagreement harder to diagnose.

use wost::arch::current::paging::AddressSpace;
use wost::arch::current::task;
use wost::mm::PhysicalAddress;
use wost::sync::SpinLock;
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
pub(crate) use call::{
    authority_for_test, refusal_report_latch_for_test, refusals, ring_enter_arms_for_test,
};
pub(crate) use call::{call, report_execution, report_refusals};
#[cfg(feature = "selftest")]
pub(crate) use construction::{
    SpawnEdge, birth_of, maximum_cleanup_ledger_is_total_for_test, refused_task_retirements,
    spawn_refusing, spawn_rollbacks, without_extended_state_for_test, without_no_execute_for_test,
};
pub(crate) use construction::{SpawnError, spawn};
use load::MAX_SEGMENTS;
pub(crate) use load::keep;
#[cfg(feature = "selftest")]
pub(crate) use load::{
    LoadError, image, load_image, refuse_cleanup_unmap_after_for_test,
    restore_cleanup_unmap_for_test,
};
pub(crate) use reap::{CollectOutcome, collect, has_zombies, orphan, zombie_slots_for_spawn};
#[cfg(feature = "selftest")]
pub(crate) use reap::{
    LastRites, collection_is_paused_for_test, collection_is_settling_for_test, last_rites_of,
    pause_collection_after_take_for_test, pause_collection_before_settlement_for_test,
    process_records_for_test, reclaimed, refused_mapping_reclaims, refused_ring_reclaims,
    refused_table_reclaims, zombie_owns_slot_for_test,
};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    gated_cross_is_waiting, release_gated_cross, run_duplicating, run_faulting, run_for_bootstrap,
    run_gated_for_wake, run_into_the_copy_window, run_measuring, run_probing, run_second,
    run_waiting, run_with_invalid_return_flags, run_with_invalid_return_stack,
};

/// Where the first user program's page of code goes: the low half, which by definition the kernel
/// does not map, split from it at bit 63 (`010` §10's LASS note).
#[cfg(feature = "selftest")]
pub(crate) const CODE: u64 = 0x0000_4000_0000_0000;

/// Where its stack goes, well away from the code so a run-off hits nothing.
pub(crate) const STACK: u64 = 0x0000_3000_0000_0000;

/// Where its ring's shared pages go: one fixed ABI address in every program's own space, which
/// costs nothing because no two programs share a table.
pub(crate) const RING: u64 = 0x0000_2000_0000_0000;

/// How far apart two programs' pages sit *within* their own spaces: page tables isolate them,
/// and this keeps their recorded addresses distinct without re-reading those tables.
const APART: u64 = 0x1_0000;

/// Bytes of the *program*'s stack, one page because it makes one call and never returns; named
/// apart from `thread::STACK_BYTES`, a *kernel* thread's 32 KiB, so two sizes never share a name.
const USER_STACK_BYTES: u64 = PAGE_SIZE;

/// The program's first four bytes: `pop rcx` and the start of the entry version check it makes
/// before either payload word. `xtask` writes them, so reading them back is about the initrd.
#[cfg(feature = "selftest")]
pub(crate) const PROGRAM: [u8; 4] = [0x59, 0x48, 0x83, 0xF9];

/// What a thread finds when it wakes in a space of its own, arranged by [`spawn`] while the slot
/// is still `Reserved` so it is there before the thread's first instruction.
#[derive(Clone, Copy, Debug)]
struct Launch {
    entry: u64,
    stack: u64,
    argument: u64,
    bootstrap_handle: u32,
    which: u64,
    root: PhysicalAddress,
}

/// One per thread slot. A leaf: nothing is called while it is held.
static LAUNCHES: SpinLock<[Option<Launch>; thread::CAPACITY]> =
    SpinLock::new([None; thread::CAPACITY]);

const MAX_REGIONS: usize = MAX_SEGMENTS + 1;

#[derive(Debug)]
struct Bootstrap {
    /// Its tables, kept so they can be given back: exactly why `bind` borrows the space.
    space: AddressSpace,
    /// The task table's one-shot authority to release these tables.
    authority: Option<BootstrapAuthority>,
    /// What was mapped into them, so the same is unmapped.
    regions: [(u64, u64); MAX_REGIONS],
    mapped: usize,
    returned_pages: usize,
    abandoned: usize,
    /// Its handle table, behind an `Arc` like [`Bootstrap::ring`]: `kind_of` takes the global
    /// arena's lock, which `lock_leaf` forbids under this table's leaf guard.
    table: alloc::sync::Arc<HandleTable>,
    /// Its ring, held because the kernel's shared pages must outlive the mapping showing them to
    /// the program; an `Arc` so serving drops `BOOTSTRAPS`' guard before reading user memory,
    /// taking other locks and waking threads.
    ring: alloc::sync::Arc<queue::Queue>,
    /// The birth handle retained for lifecycle checks.
    #[cfg(feature = "selftest")]
    observed_birth_handle: u32,
}

#[derive(Debug)]
enum BootstrapAuthority {
    Bound(task::Bound),
    Released(task::Released),
}

/// One per thread slot, alongside its [`Launch`].
static BOOTSTRAPS: SpinLock<[Option<Bootstrap>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

/// Identity of a spawned program.
#[derive(Debug)]
pub(crate) struct Spawned {
    /// The thread slot it runs in.
    pub(crate) slot: usize,
    /// The exact authority-table incarnation recorded at birth.
    #[cfg(feature = "selftest")]
    pub(crate) observed_table_incarnation: u64,
}

/// The first user program, which calls the kernel.
pub(crate) fn run() -> Result<Spawned, SpawnError> {
    spawn("user", 0, BEHAVE)
}

/// The argument that makes the program call the kernel.
const BEHAVE: u64 = 0;
