use world_tree::{ProcessStep, Progress};
use wost::arch::current::paging::{MapError, ReclaimError};
use wost::arch::current::task;

use super::{Authority, Process, ProcessRef, Space, status_of};
use crate::world::World;
use crate::{info, thread, warn};

#[derive(Debug)]
enum ReclaimRefusal {
    TaskRelease,
    Mapping(MapError),
    Tables(ReclaimError),
}

// One step of the process teardown; `Pending` leaves the process for the reaper's next round.
pub(crate) fn step(here: ProcessRef) -> Progress {
    let Some((world, process)) = here.hold() else {
        return Progress::Done;
    };
    let step = process.life.lock_leaf().step;
    let progress = match step {
        ProcessStep::Exit => exit(&world, &process),
        ProcessStep::Quiesce => quiesce(&process),
        ProcessStep::AbandonQueue => abandon(&process),
        ProcessStep::DrainTable => drain(&process),
        ProcessStep::ClearSpace => clear(&process),
        ProcessStep::ReleaseRecords => release(&process),
        ProcessStep::Released => Progress::Done,
    };
    let finished = {
        let mut life = process.life.lock_leaf();
        life.step = life.step.next(progress);
        life.step == ProcessStep::Released
    };
    if !finished {
        return Progress::Pending;
    }
    #[cfg(feature = "selftest")]
    let rites = (process.observation, process.life.lock_leaf().rites);
    drop(process);
    if !here.destroy() {
        return Progress::Pending;
    }
    #[cfg(feature = "selftest")]
    if let (observation, Some(rites)) = rites
        && let Some(observation) = world.observations.hold(observation)
    {
        observation.keep_rites(rites);
    }
    Progress::Done
}

// P0: the status is fixed once, then written where the spawner observes it.
fn exit(world: &World, process: &Process) -> Progress {
    let status = {
        let mut life = process.life.lock_leaf();
        let status = status_of(life.ending);
        match life.state.exit(status) {
            Ok(exited) => life.state = exited,
            Err(_) => return Progress::Done,
        }
        status
    };
    if let Some(observation) = world.observations.hold(process.observation) {
        observation.record(status);
    }
    Progress::Done
}

// P1: the thread has left its stack for good.
fn quiesce(process: &Process) -> Progress {
    let slot = process.life.lock_leaf().slot;
    if slot.is_none_or(thread::is_retired_off_cpu) {
        Progress::Done
    } else {
        Progress::Pending
    }
}

// P2: every queued operation ends with `CANCELLED`; a running one finishes first.
fn abandon(process: &Process) -> Progress {
    let ring = process.objects.ring;
    let quiet = ring.hold().is_none_or(|queue| {
        queue.abandon(ring);
        queue.is_quiet()
    });
    if quiet {
        Progress::Done
    } else {
        Progress::Pending
    }
}

// P3: the table and every reference its entries hold go back.
fn drain(process: &Process) -> Progress {
    #[cfg(feature = "selftest")]
    keep_rites(process);
    if process.objects.destroy() {
        Progress::Done
    } else {
        Progress::Pending
    }
}

#[cfg(feature = "selftest")]
fn keep_rites(process: &Process) {
    if process.life.lock_leaf().rites.is_some() {
        return;
    }
    let handles = process
        .objects
        .table
        .hold()
        .map_or(0, |(_, table)| table.live());
    let served = process
        .objects
        .ring
        .hold()
        .map_or(0, |ring| ring.served_count());
    let mut life = process.life.lock_leaf();
    life.rites = Some(super::Rites {
        handles,
        served,
        ending: life.ending,
    });
}

// P4: the space is taken out of the process, cleared and reclaimed; a refusal puts it back.
fn clear(process: &Process) -> Progress {
    let Some(mut space) = process.space.lock_leaf().take() else {
        return Progress::Done;
    };
    match reclaim(&mut space) {
        Ok(()) => {
            drop(space);
            Progress::Done
        }
        Err(refusal) => {
            match refusal {
                ReclaimRefusal::TaskRelease => warn!("process: task release refused; retrying"),
                ReclaimRefusal::Mapping(reason) => {
                    warn!("process: mapping cleanup refused: {reason}; retrying");
                }
                ReclaimRefusal::Tables(reason) => {
                    warn!("process: table cleanup refused: {reason}; retrying");
                }
            }
            *process.space.lock_leaf() = Some(space);
            Progress::Pending
        }
    }
}

fn reclaim(space: &mut Space) -> Result<(), ReclaimRefusal> {
    let Some(authority) = space.authority.take() else {
        panic!("a committed process always owns address-space authority");
    };
    space.authority = match authority {
        Authority::Released(released) => Some(Authority::Released(released)),
        Authority::Bound(bound) => match task::release_address_space(bound) {
            Ok(released) => Some(Authority::Released(released)),
            Err(bound) => {
                space.authority = Some(Authority::Bound(bound));
                return Err(ReclaimRefusal::TaskRelease);
            }
        },
    };
    space
        .regions
        .unmap(&mut space.space)
        .map_err(|refusal| ReclaimRefusal::Mapping(refusal.reason))?;
    let Some(Authority::Released(released)) = space.authority.take() else {
        panic!("task release was established above");
    };
    if let Err(failure) = space.space.reclaim(Some(released)) {
        let reason = failure.reason;
        let Some(released) = failure.released else {
            panic!("process cleanup lost its retry authority: {reason}");
        };
        space.authority = Some(Authority::Released(released));
        return Err(ReclaimRefusal::Tables(reason));
    }
    Ok(())
}

// P5: the thread's slot returns, and the ending becomes a boot record, once.
fn release(process: &Process) -> Progress {
    let (slot, ending) = {
        let mut life = process.life.lock_leaf();
        if let Ok(released) = life.state.release() {
            life.state = released;
        }
        (life.slot.take(), life.ending)
    };
    if let Some(slot) = slot {
        thread::release_retired(slot);
    }
    if let Some(ending) = ending {
        info!("{ending}");
    }
    Progress::Done
}
