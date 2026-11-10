use core::fmt;
#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicUsize, Ordering};

use handle_space::Rights;
use wost::arch::current::paging::{ActivateError, AddressSpace, MapError};
use wost::arch::current::task;
use wost::arch::current::{cpu, xstate};
#[cfg(feature = "selftest")]
use wost::sync::{Injection, SpinLock};
#[cfg(feature = "selftest")]
use x86_paging::{Flags, PAGE_SIZE};

#[cfg(feature = "selftest")]
use super::BEHAVE;
use super::call::cross;
#[cfg(feature = "selftest")]
use super::load::CLEANUP_REFUSAL_INJECTED;
#[cfg(feature = "selftest")]
use super::load::MAX_SEGMENTS;
use super::load::{LoadError, LoadedImage, load, place_stack, unmap_for_cleanup};
#[cfg(feature = "selftest")]
use super::reap::REFUSED_MAPPING_RECLAIMS;
use super::reap::slot_has_zombie;
use super::{
    APART, BOOTSTRAPS, Bootstrap, BootstrapAuthority, LAUNCHES, Launch, MAX_REGIONS, RING, Spawned,
    USER_STACK_BYTES,
};
use crate::handle::{HandleError, HandleTable};
#[cfg(feature = "selftest")]
use crate::object::ObjectType;
use crate::{object, queue, thread};

fn remember(
    regions: &mut [(u64, u64); MAX_REGIONS],
    used: &mut usize,
    at: u64,
    len: u64,
) -> Result<(), SpawnError> {
    let Some(slot) = regions.get_mut(*used) else {
        return Err(SpawnError::TooManyMappings);
    };
    *slot = (at, len);
    *used += 1;
    Ok(())
}

/// Whether every maximum-sized image region and its stack fit exactly once.
#[cfg(feature = "selftest")]
pub(crate) fn maximum_cleanup_ledger_is_total_for_test() -> bool {
    let mut regions = [(0, 0); MAX_REGIONS];
    let mut used = 0;
    for index in 0..MAX_SEGMENTS {
        if remember(
            &mut regions,
            &mut used,
            0x1000 + index as u64 * 0x2000,
            PAGE_SIZE,
        )
        .is_err()
        {
            return false;
        }
    }
    let stack = 0x9000_0000;
    remember(&mut regions, &mut used, stack, USER_STACK_BYTES).is_ok()
        && used == MAX_REGIONS
        && regions.get(MAX_SEGMENTS) == Some(&(stack, USER_STACK_BYTES))
        && matches!(
            remember(&mut regions, &mut used, stack + PAGE_SIZE, PAGE_SIZE),
            Err(SpawnError::TooManyMappings)
        )
}

/// A process that is not yet observable and therefore must be undone on drop.
#[derive(Debug)]
struct ProcessConstruction {
    space: Option<AddressSpace>,
    authority: thread::SpaceAuthority,
    regions: [(u64, u64); MAX_REGIONS],
    mapped: usize,
}

impl ProcessConstruction {
    fn new(space: AddressSpace) -> Self {
        Self {
            space: Some(space),
            authority: thread::SpaceAuthority::None,
            regions: [(0, 0); MAX_REGIONS],
            mapped: 0,
        }
    }

    fn space(&mut self) -> Result<&mut AddressSpace, SpawnError> {
        self.space.as_mut().ok_or(SpawnError::Incomplete)
    }

    fn remember(&mut self, at: u64, len: u64) -> Result<(), SpawnError> {
        remember(&mut self.regions, &mut self.mapped, at, len)
    }

    fn retain_authority(&mut self, authority: thread::SpaceAuthority) {
        self.authority = authority;
    }
}

impl Drop for ProcessConstruction {
    fn drop(&mut self) {
        let released = match core::mem::replace(&mut self.authority, thread::SpaceAuthority::None) {
            thread::SpaceAuthority::None => None,
            thread::SpaceAuthority::Released(released) => Some(released),
        };
        let Some(space) = self.space.as_mut() else {
            return;
        };

        #[cfg(feature = "selftest")]
        let mut observed_returned_pages = 0usize;
        while self.mapped != 0 {
            let index = self.mapped - 1;
            let (at, len) = self.regions[index];
            match unmap_for_cleanup(space, at, len) {
                Ok(returned) => {
                    #[cfg(feature = "selftest")]
                    {
                        observed_returned_pages = observed_returned_pages
                            .checked_add(returned.pages() as usize)
                            .unwrap_or_else(|| panic!("failed-spawn page accounting overflowed"));
                    }
                    drop(returned);
                    self.mapped = index;
                }
                Err(reason) => {
                    #[cfg(feature = "selftest")]
                    if CLEANUP_REFUSAL_INJECTED.swap(false, Ordering::AcqRel) {
                        REFUSED_MAPPING_RECLAIMS.fetch_add(1, Ordering::AcqRel);
                        continue;
                    }
                    panic!("failed-spawn mapping cleanup refused: {reason}");
                }
            }
        }

        match space.reclaim(released) {
            #[cfg(feature = "selftest")]
            Ok(tables) => {
                ROLLED_BACK_PAGES.fetch_add(observed_returned_pages, Ordering::AcqRel);
                ROLLED_BACK_TABLES.fetch_add(tables, Ordering::AcqRel);
            }
            #[cfg(not(feature = "selftest"))]
            Ok(_) => {}
            Err(reason) => {
                panic!("failed-spawn table cleanup refused: {reason}");
            }
        }
    }
}

#[derive(Debug)]
struct PreparedProcess {
    construction: ProcessConstruction,
    table: alloc::sync::Arc<HandleTable>,
    ring: alloc::sync::Arc<queue::Queue>,
    #[cfg(feature = "selftest")]
    observed_birth_handle: u32,
    launch: Launch,
}

#[derive(Debug)]
struct CommitOutcome {
    retained: Option<(PreparedProcess, task::Bound)>,
}

impl CommitOutcome {
    const fn committed() -> Self {
        Self { retained: None }
    }

    fn refused(process: PreparedProcess, bound: task::Bound) -> Self {
        Self {
            retained: Some((process, bound)),
        }
    }

    fn into_retained(self) -> Option<(PreparedProcess, task::Bound)> {
        self.retained
    }
}

impl PreparedProcess {
    fn commit(
        mut self,
        slot: usize,
        bound: task::Bound,
        #[cfg(feature = "selftest")] refuse: bool,
    ) -> CommitOutcome {
        #[cfg(feature = "selftest")]
        if refuse {
            return CommitOutcome::refused(self, bound);
        }
        if slot_has_zombie(slot) {
            return CommitOutcome::refused(self, bound);
        }
        assert!(
            LAUNCHES.lock_leaf().get(slot).is_some_and(Option::is_none),
            "a reserved process slot retained launch state"
        );
        assert!(
            BOOTSTRAPS
                .lock_leaf()
                .get(slot)
                .is_some_and(Option::is_none),
            "a reserved process slot retained a live owner"
        );
        let Some(space) = self.construction.space.take() else {
            return CommitOutcome::refused(self, bound);
        };
        LAUNCHES.lock_leaf()[slot] = Some(self.launch);
        BOOTSTRAPS.lock_leaf()[slot] = Some(Bootstrap {
            space,
            authority: Some(BootstrapAuthority::Bound(bound)),
            regions: self.construction.regions,
            mapped: self.construction.mapped,
            returned_pages: 0,
            abandoned: 0,
            table: self.table,
            ring: self.ring,
            #[cfg(feature = "selftest")]
            observed_birth_handle: self.observed_birth_handle,
        });
        CommitOutcome::committed()
    }

    #[cfg(feature = "selftest")]
    fn record_birth(&self, slot: usize) -> bool {
        let Some((_, ring_flags)) = self
            .construction
            .space
            .as_ref()
            .and_then(|space| space.lookup(RING))
        else {
            return false;
        };
        let Ok((bootstrap, _)) = self.table.resolve(self.observed_birth_handle) else {
            return false;
        };
        let birth = Birth {
            table: self.table.identity_for_test(),
            handles: self.table.live(),
            bootstrap: bootstrap.kind(),
            ring: self.ring.shape(),
            ring_flags,
        };
        let mut births = BIRTHS.lock_leaf();
        let Some(room) = births.get_mut(slot) else {
            return false;
        };
        *room = Some(birth);
        true
    }
}

/// Handles a process may hold at once: small enough to bound kernel memory, large enough to
/// delegate.
const HANDLE_QUOTA: usize = 64;

/// What process construction committed before the first user instruction.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Birth {
    /// The non-repeating identity of the process's authority table.
    pub(crate) table: u64,
    /// Handles in that table at commit.
    pub(crate) handles: usize,
    /// What its bootstrap entry named.
    pub(crate) bootstrap: ObjectType,
    /// The kernel-owned shape of its shared ring.
    pub(crate) ring: (u32, u32, u32),
    /// Flags on the committed ring mapping.
    pub(crate) ring_flags: Flags,
}

#[cfg(feature = "selftest")]
static BIRTHS: SpinLock<[Option<Birth>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

/// Immutable birth evidence for this exact process incarnation.
#[cfg(feature = "selftest")]
pub(crate) fn birth_of(spawned: &Spawned) -> Option<Birth> {
    BIRTHS
        .lock_leaf()
        .get(spawned.slot)
        .copied()
        .flatten()
        .filter(|birth| birth.table == spawned.observed_table_incarnation)
}

/// A completed construction step after which a check may force refusal.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SpawnEdge {
    /// The private address-space root exists.
    Space,
    /// Every image segment is mapped and recorded.
    Image,
    /// The user stack is mapped and recorded.
    Stack,
    /// The shared time page is visible.
    TimePage,
    /// The ring and its frames exist.
    Ring,
    /// The ring is mapped into the new space.
    RingMapping,
    /// The opaque bootstrap identity exists.
    Channel,
    /// The handle table has committed its quota.
    Table,
    /// The bootstrap identity is installed as the sole handle.
    Handle,
    /// The new space passed binding validation.
    Binding,
    /// A scheduler slot is reserved.
    ThreadReserved,
    /// A task record and kernel stack exist.
    ThreadFabricated,
    /// The task record names the new address space.
    ThreadBound,
    /// The reserved thread refuses its launch records before becoming runnable.
    ThreadArrangement,
}

#[cfg(feature = "selftest")]
impl SpawnEdge {
    /// Every fallible construction boundary in order.
    pub(crate) const ALL: [Self; 14] = [
        Self::Space,
        Self::Image,
        Self::Stack,
        Self::TimePage,
        Self::Ring,
        Self::RingMapping,
        Self::Channel,
        Self::Table,
        Self::Handle,
        Self::Binding,
        Self::ThreadReserved,
        Self::ThreadFabricated,
        Self::ThreadBound,
        Self::ThreadArrangement,
    ];
}

#[cfg(feature = "selftest")]
#[derive(Clone, Copy, Debug)]
enum SpawnFault {
    Never,
    At(SpawnEdge),
}

#[cfg(feature = "selftest")]
impl SpawnFault {
    const fn refuses(self, edge: SpawnEdge) -> bool {
        match self {
            Self::Never => false,
            Self::At(wanted) => wanted as u8 == edge as u8,
        }
    }

    const fn thread_point(self) -> Option<thread::SpawnPoint> {
        match self {
            Self::At(SpawnEdge::ThreadReserved) => Some(thread::SpawnPoint::Reserved),
            Self::At(SpawnEdge::ThreadFabricated) => Some(thread::SpawnPoint::Fabricated),
            Self::At(SpawnEdge::ThreadBound) => Some(thread::SpawnPoint::Bound),
            Self::Never | Self::At(_) => None,
        }
    }
}

/// Why a program could not be started.
#[derive(Debug)]
pub(crate) enum SpawnError {
    /// Boot did not admit maskable interrupts.
    NoInterruptAdmission,
    /// Neither shared-root nor paired-root isolation is admitted.
    NoUserIsolation,
    /// The CPU is not enforcing non-executable mappings.
    NoExecuteProtection,
    /// Thread switches cannot isolate architectural extended state.
    NoExtendedState,
    /// No calibrated clock can preempt or complete a timer.
    NoClock,
    /// No kernel service thread can reclaim the process after exit.
    NoServiceWorker,
    /// The kernel itself is still running on an unguarded boot stack.
    UnguardedBootStack,
    /// No address space could be built for it.
    NoSpace(MapError),
    /// The image would not load.
    Load(LoadError),
    /// No memory for its stack.
    NoStack,
    /// The space could not be bound to a thread.
    NotBindable(ActivateError),
    /// No thread could be created.
    NoThread(thread::SpawnError),
    /// An unfinished transaction lost the address space it owns.
    Incomplete,
    /// The fixed cleanup ledger cannot retain another owned mapping.
    TooManyMappings,
    /// Its bootstrap channel could not be put in its table.
    NoBootstrap(HandleError),
    /// The time page could not be shown to it.
    NoTimePage(crate::timepage::TimePageError),
    /// No memory for its ring.
    NoRing,
    /// A self-check stopped construction at one exact edge.
    #[cfg(feature = "selftest")]
    Injected(SpawnEdge),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoInterruptAdmission => {
                f.write_str("maskable interrupts are not admitted; ring 3 is closed")
            }
            Self::NoUserIsolation => {
                f.write_str("no admitted user isolation mode; ring 3 is closed")
            }
            Self::NoExecuteProtection => f.write_str("no execute protection; ring 3 is closed"),
            Self::NoExtendedState => f.write_str("no isolated extended state; ring 3 is closed"),
            Self::NoClock => f.write_str("no calibrated clock; ring 3 is closed"),
            Self::NoServiceWorker => f.write_str("no service worker; ring 3 is closed"),
            Self::UnguardedBootStack => {
                f.write_str("the boot thread has no guarded stack; ring 3 is closed")
            }
            Self::NoSpace(reason) => write!(f, "no address space: {reason}"),
            Self::Load(reason) => write!(f, "{reason}"),
            Self::NoStack => f.write_str("no memory for a stack"),
            Self::NotBindable(reason) => write!(f, "the space cannot be installed: {reason}"),
            Self::NoThread(reason) => write!(f, "no thread: {reason}"),
            Self::Incomplete => f.write_str("the process construction is incomplete"),
            Self::TooManyMappings => f.write_str("the process has too many owned mappings"),
            Self::NoBootstrap(reason) => write!(f, "no bootstrap channel: {reason}"),
            Self::NoTimePage(reason) => write!(f, "the time page: {reason}"),
            Self::NoRing => f.write_str("no memory for a ring"),
            #[cfg(feature = "selftest")]
            Self::Injected(edge) => write!(f, "construction was stopped after {edge:?}"),
        }
    }
}

impl core::error::Error for SpawnError {}

#[cfg(feature = "selftest")]
fn stop_after(fault: SpawnFault, edge: SpawnEdge) -> Result<(), SpawnError> {
    if fault.refuses(edge) {
        return Err(SpawnError::Injected(edge));
    }
    Ok(())
}

/// Start a program in an address space of its own.
///
/// The order is forced, each step making the next possible: a space sharing only the kernel's
/// higher half, the image loaded into *that* space, a stack, the binding that proves the space
/// installable, and a thread whose tables are set before anything can schedule it.
/// Until the reserved thread is runnable, [`ProcessConstruction`] owns every mapped region and
/// the tables reaching it; any refusal drops it, retiring a partly fabricated task if necessary,
/// and returns every resource before the error escapes.
pub(crate) fn spawn(name: &'static str, which: u64, argument: u64) -> Result<Spawned, SpawnError> {
    #[cfg(feature = "selftest")]
    return spawn_transaction(name, which, argument, SpawnFault::Never);
    #[cfg(not(feature = "selftest"))]
    spawn_transaction(name, which, argument)
}

fn spawn_transaction(
    name: &'static str,
    which: u64,
    argument: u64,
    #[cfg(feature = "selftest")] fault: SpawnFault,
) -> Result<Spawned, SpawnError> {
    let mut construction = ProcessConstruction::new(admitted_space()?);
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Space)?;

    let (image, stack) = map_program(
        &mut construction,
        which,
        #[cfg(feature = "selftest")]
        fault,
    )?;
    let ring = map_shared(
        &mut construction,
        #[cfg(feature = "selftest")]
        fault,
    )?;
    let root = construction.space()?.root();

    // Create the bootstrap identity before the thread can observe it.
    let bootstrap = object::bootstrap_identity();
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Channel)?;
    let table = HandleTable::new(HANDLE_QUOTA).map_err(SpawnError::NoBootstrap)?;
    #[cfg(feature = "selftest")]
    let table_identity = table.identity_for_test();
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Table)?;
    let handle = table
        .insert(bootstrap, Rights::UNIVERSAL)
        .map_err(SpawnError::NoBootstrap)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Handle)?;

    let binding = construction
        .space()?
        .bind()
        .map_err(SpawnError::NotBindable)?;
    #[cfg(feature = "selftest")]
    if let Err(reason) = stop_after(fault, SpawnEdge::Binding) {
        construction.retain_authority(thread::SpaceAuthority::Released(task::release_uninstalled(
            binding,
        )));
        return Err(reason);
    }

    let mut prepared = Some(PreparedProcess {
        construction,
        table: alloc::sync::Arc::new(table),
        #[cfg(feature = "selftest")]
        observed_birth_handle: handle,
        ring,
        launch: Launch {
            entry: image.entry(),
            stack,
            argument,
            bootstrap_handle: handle,
            which,
            root,
        },
    });
    let arrange = |slot, bound| {
        if slot >= thread::CAPACITY {
            return Err(bound);
        }
        if prepared.is_none() {
            return Err(bound);
        }
        #[cfg(feature = "selftest")]
        if !prepared
            .as_ref()
            .is_some_and(|process| process.record_birth(slot))
        {
            return Err(bound);
        }
        let Some(process) = prepared.take() else {
            return Err(bound);
        };
        match process
            .commit(
                slot,
                bound,
                #[cfg(feature = "selftest")]
                fault.refuses(SpawnEdge::ThreadArrangement),
            )
            .into_retained()
        {
            None => Ok(()),
            Some((process, bound)) => {
                prepared = Some(process);
                Err(bound)
            }
        }
    };
    #[cfg(feature = "selftest")]
    let spawned = match fault.thread_point() {
        Some(point) => thread::spawn_in_space_refusing(name, cross, binding, point, arrange),
        None => thread::spawn_in_space(name, cross, binding, arrange),
    };
    #[cfg(not(feature = "selftest"))]
    let spawned = thread::spawn_in_space(name, cross, binding, arrange);
    let slot = match spawned {
        Ok(slot) => slot,
        Err(failure) => {
            #[cfg(feature = "selftest")]
            if matches!(failure.reason, thread::SpawnError::Retirement { .. }) {
                REFUSED_TASK_RETIREMENTS.fetch_add(1, Ordering::AcqRel);
            }
            if let Some(process) = prepared.as_mut() {
                process.construction.retain_authority(failure.authority);
            }
            return Err(SpawnError::NoThread(failure.reason));
        }
    };

    Ok(Spawned {
        slot,
        #[cfg(feature = "selftest")]
        observed_table_incarnation: table_identity,
    })
}

/// Refuse in this order unless every ring 3 precondition holds, then build the admitted space.
fn admitted_space() -> Result<AddressSpace, SpawnError> {
    if !crate::maskable_interrupts_are_ready() {
        return Err(SpawnError::NoInterruptAdmission);
    }
    if !crate::user_launch_is_safe() {
        return Err(SpawnError::UnguardedBootStack);
    }
    let isolation = crate::user_isolation();
    if !no_execute_is_enabled() {
        return Err(SpawnError::NoExecuteProtection);
    }
    if !extended_state_is_isolated() {
        return Err(SpawnError::NoExtendedState);
    }
    if !crate::time::is_calibrated() {
        return Err(SpawnError::NoClock);
    }
    if !crate::queue::worker_is_ready() {
        return Err(SpawnError::NoServiceWorker);
    }
    match isolation {
        crate::UserIsolation::Closed => return Err(SpawnError::NoUserIsolation),
        crate::UserIsolation::Shared => AddressSpace::for_user(),
        crate::UserIsolation::Paired => AddressSpace::for_user_isolated(),
    }
    .map_err(SpawnError::NoSpace)
}

/// Load the image into the new space and map its stack, recording both for cleanup.
fn map_program(
    construction: &mut ProcessConstruction,
    which: u64,
    #[cfg(feature = "selftest")] fault: SpawnFault,
) -> Result<(LoadedImage, u64), SpawnError> {
    let image = load(construction.space()?, which * APART).map_err(SpawnError::Load)?;
    for (at, len) in image.regions() {
        construction.remember(at, len)?;
    }
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Image)?;

    let stack = place_stack(construction.space()?, which).ok_or(SpawnError::NoStack)?;
    construction.remember(stack, USER_STACK_BYTES)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Stack)?;
    Ok((image, stack))
}

/// Show the time page before the ring, so no process launches with a hole in its ABI, then map
/// the ring, whose pages stay the ring's and never enter the cleanup ledger.
fn map_shared(
    construction: &mut ProcessConstruction,
    #[cfg(feature = "selftest")] fault: SpawnFault,
) -> Result<alloc::sync::Arc<queue::Queue>, SpawnError> {
    crate::timepage::install(construction.space()?).map_err(SpawnError::NoTimePage)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::TimePage)?;

    let mut ring = queue::Queue::new().ok_or(SpawnError::NoRing)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Ring)?;
    alloc::sync::Arc::get_mut(&mut ring)
        .ok_or(SpawnError::Incomplete)?
        .install(construction.space()?, RING)
        .map_err(SpawnError::NoSpace)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::RingMapping)?;
    Ok(ring)
}

/// Whether the CPU will enforce every non-executable user mapping.
fn no_execute_is_enabled() -> bool {
    let enabled = cpu::Protections::enabled().nx;
    #[cfg(feature = "selftest")]
    let enabled = enabled && !REFUSE_NO_EXECUTE.armed();
    enabled
}

fn extended_state_is_isolated() -> bool {
    xstate::user_state_is_isolated()
}

#[cfg(feature = "selftest")]
static REFUSE_NO_EXECUTE: Injection = Injection::new();

/// Run a check as if the protection read-back had omitted NX.
#[cfg(feature = "selftest")]
pub(crate) fn without_no_execute_for_test<T>(check: impl FnOnce() -> T) -> T {
    REFUSE_NO_EXECUTE.with(true, check)
}

/// Run a check as if thread switches could not isolate extended state.
#[cfg(feature = "selftest")]
pub(crate) fn without_extended_state_for_test<T>(check: impl FnOnce() -> T) -> T {
    xstate::without_user_state_isolation_for_test(check)
}

/// Stop one never-run construction after `edge`.
#[cfg(feature = "selftest")]
pub(crate) fn spawn_refusing(edge: SpawnEdge) -> Result<(), SpawnError> {
    spawn_transaction("refused-user", 8, BEHAVE, SpawnFault::At(edge)).map(drop)
}

#[cfg(feature = "selftest")]
static REFUSED_TASK_RETIREMENTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static ROLLED_BACK_PAGES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static ROLLED_BACK_TABLES: AtomicUsize = AtomicUsize::new(0);

/// Prepared-task retirement first attempts which required a successful retry.
#[cfg(feature = "selftest")]
pub(crate) fn refused_task_retirements() -> usize {
    REFUSED_TASK_RETIREMENTS.load(Ordering::Acquire)
}

/// Pages and table frames returned by failed constructions.
#[cfg(feature = "selftest")]
pub(crate) fn spawn_rollbacks() -> (usize, usize) {
    (
        ROLLED_BACK_PAGES.load(Ordering::Acquire),
        ROLLED_BACK_TABLES.load(Ordering::Acquire),
    )
}
