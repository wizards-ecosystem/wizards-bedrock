use world_tree::{Admission, Limits, MAX_DEPTH, NodeRef, Path, Progress, WorldState, WorldStep};
use wost::mm::pool::{self, ArenaId, Dissolved};

use super::{DESCENDANTS, Killer, registry};
use crate::{reaper, thread};

// Frame records one dissolve call examines with interrupts off.
const DISSOLVE_CHUNK: u32 = 512;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum KillRefusal {
    Dead,
    Root,
    Full,
}

pub(crate) fn kill(node: NodeRef) -> bool {
    condemn(node, None) == Ok(true)
}

pub(crate) fn kill_for(node: NodeRef, killer: Killer) -> Result<(), KillRefusal> {
    condemn(node, Some(killer)).map(|_| ())
}

// Live to Dying once, under the world's life lock so no child joins after; the winner submits it.
fn condemn(node: NodeRef, killer: Option<Killer>) -> Result<bool, KillRefusal> {
    let world = registry::pin(node, Admission::Live).ok_or(KillRefusal::Dead)?;
    if world.path.depth() == 0 {
        return Err(KillRefusal::Root);
    }
    if let Some(killer) = killer
        && !registry::enlist(node, killer)
    {
        return Err(KillRefusal::Full);
    }
    let killed = {
        let _life = world.life.lock_leaf();
        registry::mark_dying(node)
    };
    if killed {
        pool::condemn(world.arena);
    }
    drop(world);
    if killed {
        reaper::submit_world(node);
    }
    Ok(killed)
}

// The frames the world reserves and the frames it uses.
pub(crate) fn inspect(node: NodeRef) -> Option<(usize, usize)> {
    let world = registry::pin(node, Admission::Live)?;
    pool::usage(world.arena).map(|usage| (usage.reserved(), usage.used()))
}

// One step of the world teardown; each waits only on its own threads, pins or the frame scan.
pub(crate) fn step(node: NodeRef, step: WorldStep) -> Progress {
    match step {
        WorldStep::Kill => kill_descendants(node),
        WorldStep::ExitProcesses => {
            thread::stop_world(node);
            Progress::Done
        }
        WorldStep::Quiesce => quiesce(node),
        // What these release is held only by the world's processes, whose teardowns ran in Quiesce;
        // endpoints are destroyed at Detach, once no other world can pin them.
        WorldStep::DestroyObjects
        | WorldStep::AbandonQueuesAndTables
        | WorldStep::ClearSpaces
        | WorldStep::RevokeMappers
        | WorldStep::Reclaim
        | WorldStep::Finished => Progress::Done,
        WorldStep::ClosePins => {
            if registry::close(node) == 0 {
                Progress::Done
            } else {
                Progress::Pending
            }
        }
        WorldStep::Detach => detach(node),
        WorldStep::Dissolve => {
            let Some(remains) = registry::remains(node) else {
                panic!("world {} dissolved without its remains", node.node);
            };
            let progress = dissolve(&remains.path, remains.arena);
            if progress == Progress::Done {
                for killer in remains.killers.into_iter().flatten() {
                    killer.queue.complete_kill(killer.pending, killer.id);
                }
            }
            progress
        }
    }
}

// The descendant set is frozen once the world is Dying; each descendant tears down by itself, and
// one not yet published is doomed instead, so its creation unwinds.
fn kill_descendants(node: NodeRef) -> Progress {
    let Some(world) = registry::hold(node) else {
        panic!("dying world {} lost its record", node.node);
    };
    let mut descendants = [None; DESCENDANTS];
    {
        let life = world.life.lock_leaf();
        for (position, slot) in (0..life.descendants.count()).zip(&mut descendants) {
            *slot = life.descendants.get(position);
        }
    }
    drop(world);
    for descendant in descendants.into_iter().flatten() {
        if !registry::doom(descendant) {
            kill(descendant);
        }
    }
    Progress::Done
}

fn quiesce(node: NodeRef) -> Progress {
    let stopped = thread::stop_world(node);
    let released = registry::hold(node)
        .is_none_or(|world| world.processes.is_empty() && world.life.lock_leaf().spawns == 0);
    if stopped && released {
        Progress::Done
    } else {
        Progress::Pending
    }
}

// Leaves every ancestor's set; the nearest one still holding a set gets back this world's own slot
// and unspent budgets, and each descendant returns its own share when it detaches.
fn detach(node: NodeRef) -> Progress {
    let Some(world) = registry::take(node) else {
        return Progress::Pending;
    };
    crate::channel::dissolve(&world);
    let unspent = world.life.lock_leaf().budgets;
    let mut share = Some(Limits {
        descendants: unspent.descendants(),
        threads: unspent.threads(),
        handles: world.limits.handles,
    });
    for distance in 0..world.path.depth() {
        let Some(ancestor) = world
            .path
            .nearest(distance)
            .and_then(|ancestor| registry::pin(ancestor, Admission::Teardown))
        else {
            continue;
        };
        let removed = ancestor.life.lock_leaf().descendants.remove(node);
        assert!(
            removed.is_ok(),
            "world {} was missing from an ancestor",
            node.node
        );
        if let Some(share) = share.take() {
            super::create::credit(&ancestor, share);
        }
    }
    world.release();
    registry::reclaim(node);
    Progress::Done
}

// Chunk by chunk, interrupts on between them; `Pending` only while no live heir could be pinned.
pub(super) fn dissolve(path: &Path, arena: ArenaId) -> Progress {
    loop {
        match dissolve_chunk(path, arena) {
            Some(Dissolved::Progress) => {}
            Some(Dissolved::Done) => return Progress::Done,
            None => return Progress::Pending,
        }
    }
}

// The heir is the nearest live ancestor, chosen again for every chunk.
fn dissolve_chunk(path: &Path, arena: ArenaId) -> Option<Dissolved> {
    let mut states = [WorldState::Free; MAX_DEPTH as usize];
    for (distance, state) in (0..path.depth()).zip(&mut states) {
        *state = path
            .nearest(distance)
            .and_then(registry::state)
            .unwrap_or(WorldState::Free);
    }
    let heir = path
        .heir(&states)
        .and_then(|heir| registry::pin(heir, Admission::Live))?;
    match pool::dissolve(arena, heir.arena, DISSOLVE_CHUNK) {
        Ok(dissolved) => Some(dissolved),
        Err(refused) => panic!("a dead world's arena refused to dissolve: {refused}"),
    }
}
