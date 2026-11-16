use boot_record::Record;
use wost::arch::current::paging::{MapError, ReclaimError};
use wost::arch::current::task;
use wost::sync::{Leaf, SpinLock};

use super::{BOOTSTRAPS, Bootstrap, BootstrapAuthority, LAUNCHES};
use crate::queue::{self, QueueRef};
use crate::{info, thread, warn};

#[derive(Debug)]
enum ReclaimRefusal {
    TaskRelease,
    Mapping(MapError),
    Tables(ReclaimError),
}

fn reclaim(bootstrap: &mut Bootstrap) -> Result<(), ReclaimRefusal> {
    let Some(authority) = bootstrap.authority.take() else {
        panic!("a zombie always owns address-space authority");
    };
    bootstrap.authority = match authority {
        BootstrapAuthority::Released(released) => Some(BootstrapAuthority::Released(released)),
        BootstrapAuthority::Bound(bound) => match task::release_address_space(bound) {
            Ok(released) => Some(BootstrapAuthority::Released(released)),
            Err(bound) => {
                bootstrap.authority = Some(BootstrapAuthority::Bound(bound));
                return Err(ReclaimRefusal::TaskRelease);
            }
        },
    };

    bootstrap
        .regions
        .unmap(&mut bootstrap.space)
        .map_err(|refusal| ReclaimRefusal::Mapping(refusal.reason))?;

    let Some(BootstrapAuthority::Released(released)) = bootstrap.authority.take() else {
        unreachable!("task release was established above");
    };
    if let Err(failure) = bootstrap.space.reclaim(Some(released)) {
        let reason = failure.reason;
        let Some(released) = failure.released else {
            panic!("zombie cleanup lost its retry authority: {reason}");
        };
        bootstrap.authority = Some(BootstrapAuthority::Released(released));
        return Err(ReclaimRefusal::Tables(reason));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CollectOutcome {
    Empty,
    Retained,
    Reclaimed,
}

impl CollectOutcome {
    pub(crate) const fn is_reclaimed(self) -> bool {
        matches!(self, Self::Reclaimed)
    }
}

// How each running program ended, set by its own thread just before it retires.
static ENDINGS: SpinLock<Leaf, [Option<Record>; thread::CAPACITY]> =
    SpinLock::new([None; thread::CAPACITY]);

pub(crate) fn record_ending(ending: Record) {
    if let Some(room) = ENDINGS.lock_leaf().get_mut(task::current()) {
        *room = Some(ending);
    }
}

#[derive(Debug)]
struct ZombieTable {
    owners: [Option<Bootstrap>; thread::CAPACITY],
    endings: [Option<Record>; thread::CAPACITY],
    settling: [bool; thread::CAPACITY],
    collecting: [bool; thread::CAPACITY],
}

impl ZombieTable {
    const fn new() -> Self {
        Self {
            owners: [const { None }; thread::CAPACITY],
            endings: [None; thread::CAPACITY],
            settling: [false; thread::CAPACITY],
            collecting: [false; thread::CAPACITY],
        }
    }
}

static ZOMBIES: SpinLock<Leaf, ZombieTable> = SpinLock::new(ZombieTable::new());

#[cfg(feature = "selftest")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct LastRites {
    pub(crate) table: u64,
    pub(crate) handles: usize,
    pub(crate) served: usize,
    pub(crate) ending: Option<Record>,
}

#[cfg(feature = "selftest")]
static RITES: SpinLock<Leaf, [Option<LastRites>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

#[cfg(feature = "selftest")]
pub(crate) fn last_rites_of(slot: usize) -> Option<LastRites> {
    RITES.lock_leaf().get(slot).copied().flatten()
}

// On the retiring thread, interrupts off: no allocation, no blocking, no touching its own space.
pub(crate) fn orphan(slot: usize) {
    let taken = BOOTSTRAPS.lock_leaf().get_mut(slot).and_then(Option::take);
    let stored = taken.is_some();
    let ending = ENDINGS.lock_leaf().get_mut(slot).and_then(Option::take);
    if let Some(bootstrap) = taken {
        #[cfg(feature = "selftest")]
        {
            let rites = LastRites {
                table: bootstrap.objects.table.identity_for_test(),
                handles: bootstrap
                    .objects
                    .table
                    .hold()
                    .map_or(0, |(_, table)| table.live()),
                served: bootstrap
                    .objects
                    .ring
                    .hold()
                    .map_or(0, |ring| ring.served_count()),
                ending,
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
        zombies.endings[slot] = ending;
    }
    // Cleared, so a slot reused by a later program never finds the previous one's entry point.
    let mut launches = LAUNCHES.lock_leaf();
    let Some(room) = launches.get_mut(slot) else {
        panic!("a retiring process slot remains in range");
    };
    *room = None;
    drop(launches);
    if stored {
        queue::request_reap();
    }
}

pub(crate) fn collect() -> CollectOutcome {
    // The zombie owns the ring until it is quiet, so running file work never meets reused pages.
    let (found, another_collector) = {
        let mut zombies = ZOMBIES.lock_leaf();
        let another_collector = zombies.settling.iter().any(|owned| *owned)
            || zombies.collecting.iter().any(|owned| *owned);
        let found = (0..thread::CAPACITY).find_map(|slot| {
            if zombies.settling[slot] {
                return None;
            }
            let bootstrap = zombies.owners[slot].as_ref()?;
            Some((slot, bootstrap.objects.ring))
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

    // Every outstanding operation completes while the zombie still owns the ring's pages.
    let quiet = ring.hold().is_none_or(|queue| {
        queue.abandon(ring);
        queue.is_quiet()
    });
    if !quiet {
        return retain_unsettled(slot, ring);
    }
    let bootstrap = match take_settled(slot, ring) {
        Ok(bootstrap) => bootstrap,
        Err(outcome) => return outcome,
    };
    settle(slot, bootstrap)
}

fn retain_unsettled(slot: usize, ring: QueueRef) -> CollectOutcome {
    let broken = {
        let mut zombies = ZOMBIES.lock_leaf();
        let settling = zombies.settling.get(slot).copied().unwrap_or(false);
        let collecting = zombies.collecting.get(slot).copied().unwrap_or(false);
        let broken = match zombies.owners.get(slot).and_then(Option::as_ref) {
            None => Some("the unsettled zombie left its slot"),
            Some(bootstrap) if bootstrap.objects.ring != ring => {
                Some("a zombie changed owners during settlement")
            }
            Some(_) if !settling || collecting => {
                Some("a zombie lost exclusive settlement ownership")
            }
            Some(_) => None,
        };
        if broken.is_none() {
            zombies.settling[slot] = false;
        }
        broken
    };
    if let Some(broken) = broken {
        panic!("{broken}");
    }
    warn!("user: ring cleanup refused; the zombie remains owned");
    CollectOutcome::Retained
}

// Taken out under the guard and reclaimed outside it, because reclaiming reaches the allocator.
fn take_settled(slot: usize, ring: QueueRef) -> Result<Bootstrap, CollectOutcome> {
    let mut zombies = ZOMBIES.lock_leaf();
    let settling = zombies.settling.get(slot).copied().unwrap_or(false);
    let Some(room) = zombies.owners.get_mut(slot) else {
        panic!("a selected zombie slot remains in range");
    };
    let Some(current) = room.as_ref() else {
        return Err(if zombies.collecting[slot] {
            CollectOutcome::Retained
        } else {
            CollectOutcome::Empty
        });
    };
    if current.objects.ring != ring {
        return Err(CollectOutcome::Retained);
    }
    if !settling {
        return Err(CollectOutcome::Retained);
    }
    let Some(bootstrap) = room.take() else {
        unreachable!("the retained zombie was checked above");
    };
    zombies.settling[slot] = false;
    zombies.collecting[slot] = true;
    Ok(bootstrap)
}

fn settle(slot: usize, mut bootstrap: Bootstrap) -> CollectOutcome {
    // A hold that outlived the queue's last operation refuses this once; the next collect retries.
    let retained = if !bootstrap.objects.destroy() {
        warn!("user: a table or ring is still borrowed; the zombie remains owned");
        Some(bootstrap)
    } else {
        match reclaim(&mut bootstrap) {
            Ok(()) => {
                drop(bootstrap);
                None
            }
            Err(refusal) => {
                match refusal {
                    ReclaimRefusal::TaskRelease => {
                        warn!("user: task release refused; the zombie remains owned");
                    }
                    ReclaimRefusal::Mapping(reason) => {
                        warn!("user: mapping cleanup refused: {reason}; the zombie remains owned");
                    }
                    ReclaimRefusal::Tables(reason) => {
                        warn!("user: table cleanup refused: {reason}; the zombie remains owned");
                    }
                }
                Some(bootstrap)
            }
        }
    };
    let outcome = if retained.is_some() {
        CollectOutcome::Retained
    } else {
        CollectOutcome::Reclaimed
    };
    let mut zombies = ZOMBIES.lock_leaf();
    // Written once, when the program is gone, and from this thread, which may take the console.
    let ending = if outcome.is_reclaimed() {
        zombies.endings[slot].take()
    } else {
        None
    };
    let collecting = zombies.collecting.get(slot).copied().unwrap_or(false);
    let Some(room) = zombies.owners.get_mut(slot) else {
        panic!("a collected zombie's slot remains in range");
    };
    assert!(
        room.is_none() && collecting,
        "a collecting zombie cannot change owners"
    );
    *room = retained;
    zombies.collecting[slot] = false;
    drop(zombies);
    if let Some(ending) = ending {
        info!("{ending}");
    }
    outcome
}

pub(crate) fn has_zombies() -> bool {
    let zombies = ZOMBIES.lock_leaf();
    zombies.owners.iter().any(Option::is_some)
        || zombies.settling.iter().any(|owned| *owned)
        || zombies.collecting.iter().any(|owned| *owned)
}

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
