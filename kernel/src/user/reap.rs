#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use wost::arch::current::paging::{MapError, ReclaimError};
use wost::arch::current::task;
use wost::sync::SpinLock;

use super::{BOOTSTRAPS, Bootstrap, BootstrapAuthority, LAUNCHES};
#[cfg(feature = "selftest")]
use crate::object::ObjectType;
#[cfg(feature = "selftest")]
use crate::queue;
use crate::{thread, warn};

enum ReclaimRefusal {
    TaskRelease,
    Mapping(MapError),
    Tables(ReclaimError),
}

struct ReclaimRetention {
    refusal: ReclaimRefusal,
    #[cfg(feature = "selftest")]
    observed_returned_pages: usize,
}

struct ReclaimSuccess {
    reclaimed: Reclaimed,
    #[cfg(feature = "selftest")]
    observed_returned_pages: usize,
}

fn reclaim(
    bootstrap: &mut Bootstrap,
    abandoned: usize,
) -> Result<ReclaimSuccess, ReclaimRetention> {
    bootstrap.abandoned = bootstrap
        .abandoned
        .checked_add(abandoned)
        .unwrap_or_else(|| panic!("zombie abandonment accounting overflowed"));
    let Some(authority) = bootstrap.authority.take() else {
        panic!("a zombie always owns address-space authority");
    };
    bootstrap.authority = match authority {
        BootstrapAuthority::Released(released) => Some(BootstrapAuthority::Released(released)),
        BootstrapAuthority::Bound(bound) => match task::release_address_space(bound) {
            Ok(released) => Some(BootstrapAuthority::Released(released)),
            Err(bound) => {
                bootstrap.authority = Some(BootstrapAuthority::Bound(bound));
                return Err(ReclaimRetention {
                    refusal: ReclaimRefusal::TaskRelease,
                    #[cfg(feature = "selftest")]
                    observed_returned_pages: 0,
                });
            }
        },
    };

    #[cfg(feature = "selftest")]
    let mut observed_returned_pages = 0usize;
    while bootstrap.mapped != 0 {
        let index = bootstrap.mapped - 1;
        let (at, len) = bootstrap.regions[index];
        match bootstrap.space.unmap(at, len) {
            Ok(returned) => {
                let pages = returned.pages() as usize;
                drop(returned);
                bootstrap.mapped = index;
                bootstrap.returned_pages = bootstrap
                    .returned_pages
                    .checked_add(pages)
                    .unwrap_or_else(|| panic!("zombie returned-page accounting overflowed"));
                #[cfg(feature = "selftest")]
                {
                    observed_returned_pages = observed_returned_pages
                        .checked_add(pages)
                        .unwrap_or_else(|| {
                            panic!("one reclaim's returned-page accounting overflowed")
                        });
                }
            }
            Err(reason) => {
                return Err(ReclaimRetention {
                    refusal: ReclaimRefusal::Mapping(reason),
                    #[cfg(feature = "selftest")]
                    observed_returned_pages,
                });
            }
        }
    }

    let Some(BootstrapAuthority::Released(released)) = bootstrap.authority.take() else {
        unreachable!("task release was established above");
    };
    let tables = match bootstrap.space.reclaim(Some(released)) {
        Ok(tables) => tables,
        Err(failure) => {
            let reason = failure.reason;
            let Some(released) = failure.released else {
                panic!("zombie cleanup lost its retry authority: {reason}");
            };
            bootstrap.authority = Some(BootstrapAuthority::Released(released));
            return Err(ReclaimRetention {
                refusal: ReclaimRefusal::Tables(reason),
                #[cfg(feature = "selftest")]
                observed_returned_pages,
            });
        }
    };

    let reclaimed = Reclaimed {
        pages: bootstrap.returned_pages,
        tables,
        abandoned: bootstrap.abandoned,
    };
    Ok(ReclaimSuccess {
        reclaimed,
        #[cfg(feature = "selftest")]
        observed_returned_pages,
    })
}

/// What one reclaim gave back.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) struct Reclaimed {
    /// Frames the program's own mappings held.
    pub(crate) pages: usize,
    /// Frames its page tables were built from.
    pub(crate) tables: usize,
    /// Operations that were still in flight and were ended.
    pub(crate) abandoned: usize,
}

/// What one bounded collection attempt observed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CollectOutcome {
    /// No zombie owner was waiting.
    Empty,
    /// A zombie remains owned and needs a later retry.
    Retained,
    /// One zombie was reclaimed completely.
    Reclaimed(Reclaimed),
}

impl CollectOutcome {
    pub(crate) const fn is_some(self) -> bool {
        matches!(self, Self::Reclaimed(_))
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn is_some_and(self, check: impl FnOnce(Reclaimed) -> bool) -> bool {
        match self {
            Self::Reclaimed(reclaimed) => check(reclaimed),
            Self::Empty | Self::Retained => false,
        }
    }
}

struct ZombieTable {
    owners: [Option<Bootstrap>; thread::CAPACITY],
    settling: [bool; thread::CAPACITY],
    collecting: [bool; thread::CAPACITY],
}

impl ZombieTable {
    const fn new() -> Self {
        Self {
            owners: [const { None }; thread::CAPACITY],
            settling: [false; thread::CAPACITY],
            collecting: [false; thread::CAPACITY],
        }
    }
}

/// Programs whose threads are gone and whose resources are not.
static ZOMBIES: SpinLock<ZombieTable> = SpinLock::new(ZombieTable::new());

/// What every reclaim so far gave back.
#[cfg(feature = "selftest")]
static RECLAIMED_PAGES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static RECLAIMED_TABLES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
pub(super) static REFUSED_MAPPING_RECLAIMS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static REFUSED_TABLE_RECLAIMS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static REFUSED_RING_RECLAIMS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
static PAUSE_COLLECTION_AFTER_TAKE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static COLLECTION_IS_PAUSED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static PAUSE_COLLECTION_BEFORE_SETTLEMENT: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static COLLECTION_IS_SETTLING: AtomicBool = AtomicBool::new(false);

/// What a program held when it died, for machine checks.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct LastRites {
    /// The non-repeating table identity of the program that left this record.
    pub(crate) table: u64,
    /// Handles in its table at the end.
    pub(crate) handles: usize,
    /// What its one bootstrap handle named.
    pub(crate) bootstrap: Option<ObjectType>,
    /// Completions its ring posted.
    pub(crate) served: usize,
    /// The first submission it wrote, and the completion that answered it.
    pub(crate) submitted: queue::Submission,
    pub(crate) completed: queue::Completion,
}

#[cfg(feature = "selftest")]
static RITES: SpinLock<[Option<LastRites>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

/// What the program that last held `slot` left behind.
#[cfg(feature = "selftest")]
pub(crate) fn last_rites_of(slot: usize) -> Option<LastRites> {
    RITES.lock_leaf().get(slot).copied().flatten()
}

/// Mark `slot`'s program dead, without freeing anything.
///
/// Called from trap context on the retiring thread itself, so it must not
/// allocate, block, or touch the space it is running on. Moving one `Option`
/// between two arrays does none of those.
pub(crate) fn orphan(slot: usize) {
    let taken = BOOTSTRAPS.lock_leaf().get_mut(slot).and_then(Option::take);
    let stored = taken.is_some();
    if let Some(bootstrap) = taken {
        #[cfg(feature = "selftest")]
        {
            let rites = LastRites {
                table: bootstrap.table.identity_for_test(),
                handles: bootstrap.table.live(),
                bootstrap: bootstrap
                    .table
                    .kind_of(bootstrap.observed_birth_handle)
                    .ok(),
                served: bootstrap.ring.served_count(),
                submitted: bootstrap.ring.read_submission(0),
                completed: bootstrap.ring.read_completion(0),
            };
            let mut rites_table = RITES.lock_leaf();
            let Some(room) = rites_table.get_mut(slot) else {
                panic!("a retiring process slot remains in range");
            };
            *room = Some(rites);
        }

        let mut zombies = ZOMBIES.lock_leaf();
        let settling = zombies.settling.get(slot).copied().unwrap_or(false);
        let collecting = zombies.collecting.get(slot).copied().unwrap_or(false);
        let Some(room) = zombies.owners.get_mut(slot) else {
            panic!("a retiring process slot remains in range");
        };
        assert!(
            room.is_none() && !settling && !collecting,
            "a process slot cannot orphan twice"
        );
        *room = Some(bootstrap);
    }
    // Cleared, so a slot reused by a later program never finds the previous
    // one's entry point (D-117).
    let mut launches = LAUNCHES.lock_leaf();
    let Some(room) = launches.get_mut(slot) else {
        panic!("a retiring process slot remains in range");
    };
    *room = None;
    drop(launches);
    if stored {
        crate::queue::request_reap();
    }
}

/// Collect one dead program's resources, if there are any.
///
/// One per call rather than all of them: this runs where the machine had
/// nothing better to do, and a loop that frees sixteen processes with
/// interrupts on is a loop that can be preempted halfway through anyway. One
/// at a time makes that unremarkable instead of something to reason about.
///
pub(crate) fn collect() -> CollectOutcome {
    // The owner stays in the zombie table until its ring is quiet, so a
    // refused terminal completion retries rather than reusing live pages (D-192).
    let (found, another_collector) = {
        let mut zombies = ZOMBIES.lock_leaf();
        let another_collector = zombies.settling.iter().any(|owned| *owned)
            || zombies.collecting.iter().any(|owned| *owned);
        let found = (0..thread::CAPACITY).find_map(|slot| {
            if zombies.settling[slot] {
                return None;
            }
            let bootstrap = zombies.owners[slot].as_ref()?;
            Some((slot, alloc::sync::Arc::clone(&bootstrap.ring)))
        });
        if let Some((slot, _)) = found.as_ref() {
            zombies.settling[*slot] = true;
        }
        (found, another_collector)
    };
    let Some((slot, ring)) = found else {
        return if another_collector {
            CollectOutcome::Retained
        } else {
            CollectOutcome::Empty
        };
    };

    #[cfg(feature = "selftest")]
    {
        COLLECTION_IS_SETTLING.store(true, Ordering::Release);
        while PAUSE_COLLECTION_BEFORE_SETTLEMENT.load(Ordering::Acquire) {
            crate::thread::yield_now(&mut crate::thread::ThreadContext::require());
        }
        COLLECTION_IS_SETTLING.store(false, Ordering::Release);
    }

    // Before the ring's pages go anywhere: every outstanding operation gets
    // its terminal completion while the zombie still owns them (D-167, D-192).
    let abandoned = ring.abandon();
    if !ring.is_quiet() {
        let recorded = {
            let mut zombies = ZOMBIES.lock_leaf();
            let settling = zombies.settling.get(slot).copied().unwrap_or(false);
            let collecting = zombies.collecting.get(slot).copied().unwrap_or(false);
            let recorded = match zombies.owners.get_mut(slot).and_then(Option::as_mut) {
                None => 0,
                Some(bootstrap) if !alloc::sync::Arc::ptr_eq(&bootstrap.ring, &ring) => 1,
                Some(_) if !settling || collecting => 2,
                Some(bootstrap) => match bootstrap.abandoned.checked_add(abandoned) {
                    Some(total) => {
                        bootstrap.abandoned = total;
                        4
                    }
                    None => 3,
                },
            };
            if recorded == 4 {
                zombies.settling[slot] = false;
            }
            recorded
        };
        match recorded {
            4 => {}
            0 => panic!("the unsettled zombie left its slot"),
            1 => panic!("a zombie changed owners during settlement"),
            2 => panic!("a zombie lost exclusive settlement ownership"),
            3 => panic!("zombie abandonment accounting overflowed"),
            _ => unreachable!(),
        }
        warn!("user: ring cleanup refused; the zombie remains owned");
        #[cfg(feature = "selftest")]
        REFUSED_RING_RECLAIMS.fetch_add(1, Ordering::AcqRel);
        return CollectOutcome::Retained;
    }
    // Taken out under the guard and reclaimed outside it: `reclaim` frees
    // frames, which reaches the allocator, and this lock is a leaf (D-85).
    let bootstrap = {
        let mut zombies = ZOMBIES.lock_leaf();
        let settling = zombies.settling.get(slot).copied().unwrap_or(false);
        let Some(room) = zombies.owners.get_mut(slot) else {
            panic!("a selected zombie slot remains in range");
        };
        let Some(current) = room.as_ref() else {
            return if zombies.collecting[slot] {
                CollectOutcome::Retained
            } else {
                CollectOutcome::Empty
            };
        };
        if !alloc::sync::Arc::ptr_eq(&current.ring, &ring) {
            return CollectOutcome::Retained;
        }
        if !settling {
            return CollectOutcome::Retained;
        }
        let Some(bootstrap) = room.take() else {
            unreachable!("the retained zombie was checked above");
        };
        zombies.settling[slot] = false;
        zombies.collecting[slot] = true;
        bootstrap
    };
    drop(ring);
    #[cfg(feature = "selftest")]
    {
        COLLECTION_IS_PAUSED.store(true, Ordering::Release);
        while PAUSE_COLLECTION_AFTER_TAKE.load(Ordering::Acquire) {
            crate::thread::yield_now(&mut crate::thread::ThreadContext::require());
        }
        COLLECTION_IS_PAUSED.store(false, Ordering::Release);
    }
    let mut bootstrap = bootstrap;
    match reclaim(&mut bootstrap, abandoned) {
        Ok(success) => {
            drop(bootstrap);
            #[cfg(feature = "selftest")]
            RECLAIMED_PAGES.fetch_add(success.observed_returned_pages, Ordering::AcqRel);
            #[cfg(feature = "selftest")]
            RECLAIMED_TABLES.fetch_add(success.reclaimed.tables, Ordering::AcqRel);
            let mut zombies = ZOMBIES.lock_leaf();
            let Some(room) = zombies.owners.get_mut(slot) else {
                panic!("a collected zombie's slot remains in range");
            };
            assert!(
                room.is_none() && zombies.collecting[slot],
                "a collecting zombie cannot change owners"
            );
            zombies.collecting[slot] = false;
            CollectOutcome::Reclaimed(success.reclaimed)
        }
        Err(retention) => {
            #[cfg(feature = "selftest")]
            RECLAIMED_PAGES.fetch_add(retention.observed_returned_pages, Ordering::AcqRel);
            match retention.refusal {
                ReclaimRefusal::TaskRelease => {
                    warn!("user: task release refused; the zombie remains owned");
                }
                ReclaimRefusal::Mapping(reason) => {
                    #[cfg(feature = "selftest")]
                    REFUSED_MAPPING_RECLAIMS.fetch_add(1, Ordering::AcqRel);
                    warn!("user: mapping cleanup refused: {reason}; the zombie remains owned");
                }
                ReclaimRefusal::Tables(reason) => {
                    #[cfg(feature = "selftest")]
                    REFUSED_TABLE_RECLAIMS.fetch_add(1, Ordering::AcqRel);
                    warn!("user: table cleanup refused: {reason}; the zombie remains owned");
                }
            }
            let mut zombies = ZOMBIES.lock_leaf();
            let collecting = zombies.collecting.get(slot).copied().unwrap_or(false);
            let Some(room) = zombies.owners.get_mut(slot) else {
                panic!("a collected zombie's slot remains in range");
            };
            assert!(
                room.is_none() && collecting,
                "a collecting zombie cannot change owners"
            );
            *room = Some(bootstrap);
            zombies.collecting[slot] = false;
            CollectOutcome::Retained
        }
    }
}

#[cfg(feature = "selftest")]
pub(crate) fn pause_collection_after_take_for_test(paused: bool) {
    PAUSE_COLLECTION_AFTER_TAKE.store(paused, Ordering::Release);
}

#[cfg(feature = "selftest")]
pub(crate) fn collection_is_paused_for_test() -> bool {
    COLLECTION_IS_PAUSED.load(Ordering::Acquire)
}

/// Pause one collector after it owns settlement but before it touches the ring.
#[cfg(feature = "selftest")]
pub(crate) fn pause_collection_before_settlement_for_test(paused: bool) {
    PAUSE_COLLECTION_BEFORE_SETTLEMENT.store(paused, Ordering::Release);
}

/// Whether one collector exclusively owns a zombie's settlement pass.
#[cfg(feature = "selftest")]
pub(crate) fn collection_is_settling_for_test() -> bool {
    COLLECTION_IS_SETTLING.load(Ordering::Acquire)
}

/// Whether any program is waiting to be collected.
pub(crate) fn has_zombies() -> bool {
    let zombies = ZOMBIES.lock_leaf();
    zombies.owners.iter().any(Option::is_some)
        || zombies.settling.iter().any(|owned| *owned)
        || zombies.collecting.iter().any(|owned| *owned)
}

/// Scheduler slots which still have a retained process owner.
pub(crate) fn zombie_slots_for_spawn() -> [bool; thread::CAPACITY] {
    let zombies = ZOMBIES.lock_leaf();
    core::array::from_fn(|slot| {
        zombies.owners[slot].is_some() || zombies.settling[slot] || zombies.collecting[slot]
    })
}

pub(super) fn slot_has_zombie(slot: usize) -> bool {
    let zombies = ZOMBIES.lock_leaf();
    zombies.owners.get(slot).is_some_and(Option::is_some)
        || zombies.settling.get(slot).is_some_and(|owned| *owned)
        || zombies.collecting.get(slot).is_some_and(|owned| *owned)
}

/// Whether a process owner still excludes this scheduler slot.
#[cfg(feature = "selftest")]
pub(crate) fn zombie_owns_slot_for_test(slot: usize) -> bool {
    slot_has_zombie(slot)
}

/// Frames given back by every collection so far.
#[cfg(feature = "selftest")]
pub(crate) fn reclaimed() -> (usize, usize) {
    (
        RECLAIMED_PAGES.load(Ordering::Acquire),
        RECLAIMED_TABLES.load(Ordering::Acquire),
    )
}

/// Address-space table cleanups refused so far.
#[cfg(feature = "selftest")]
pub(crate) fn refused_table_reclaims() -> usize {
    REFUSED_TABLE_RECLAIMS.load(Ordering::Acquire)
}

/// Owned-mapping cleanups refused so far.
#[cfg(feature = "selftest")]
pub(crate) fn refused_mapping_reclaims() -> usize {
    REFUSED_MAPPING_RECLAIMS.load(Ordering::Acquire)
}

/// Zombie collections retained because terminal ring work could not settle.
#[cfg(feature = "selftest")]
pub(crate) fn refused_ring_reclaims() -> usize {
    REFUSED_RING_RECLAIMS.load(Ordering::Acquire)
}

/// Launch, process and zombie records currently occupied.
#[cfg(feature = "selftest")]
pub(crate) fn process_records_for_test() -> (usize, usize, usize) {
    let launches = LAUNCHES.lock_leaf().iter().flatten().count();
    let bootstraps = BOOTSTRAPS.lock_leaf().iter().flatten().count();
    let zombies = ZOMBIES.lock_leaf();
    let zombies = zombies.owners.iter().flatten().count()
        + zombies.collecting.iter().filter(|owned| **owned).count();
    (launches, bootstraps, zombies)
}
