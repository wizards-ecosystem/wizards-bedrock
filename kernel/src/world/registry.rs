use core::ops::Deref;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use world_tree::{Admission, Limits, NodeRef, Path, Reclaimed, WorldEvent, WorldState};
use wost::mm::pool::{self, ArenaId, ColourMask, FrameKind, Refused};
use wost::mm::slab::{Hold, Removal};
use wost::sync::{Leaf, PinGuard, SpinLock};

use super::{Killer, Time, World, WorldKey};
use crate::arena::{Local, Pool};
use crate::rank;
use crate::thread::ThreadContext;
use crate::timepage::WorldClock;

const CAPACITY: usize = pool::capacity() as usize;

const STATE_SHIFT: u32 = 32;

const PINS: u64 = (1 << STATE_SHIFT) - 1;

// `WorldKill` calls waiting on one world at once; one more is refused `NO_ROOM`.
const KILLERS: usize = 8;

// What a teardown still needs once the record is gone: where the frames go and whose they are.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Remains {
    pub(crate) path: Path,
    pub(crate) arena: ArenaId,
    pub(crate) killers: [Option<Killer>; KILLERS],
}

// One entry per arena node; a world's record is the only object in its node's record pool.
#[derive(Debug)]
struct Entry {
    generation: AtomicU32,
    word: AtomicU64,
    record: AtomicU64,
    records: Pool<World, 1>,
    // Beside the record, since threads and queues of the world name it for as long as they live.
    clock: WorldClock,
    remains: SpinLock<Leaf, Option<Remains>>,
    killers: SpinLock<Leaf, [Option<Killer>; KILLERS]>,
}

impl Entry {
    const fn free() -> Self {
        Self {
            generation: AtomicU32::new(0),
            word: AtomicU64::new(encode(WorldState::Free)),
            record: AtomicU64::new(0),
            records: Pool::new(),
            clock: WorldClock::new(),
            remains: SpinLock::new(None),
            killers: SpinLock::new([None; KILLERS]),
        }
    }

    fn state(&self) -> WorldState {
        decode(self.word.load(Ordering::SeqCst))
    }

    fn is(&self, node: NodeRef) -> bool {
        self.generation.load(Ordering::SeqCst) == node.generation
    }

    fn record(&self) -> Local<World> {
        Local::from_bits(self.record.load(Ordering::Relaxed))
    }

    // Moves the state and keeps the pin count; `false` unless the state was `from`.
    fn turn(&self, from: WorldState, to: WorldState) -> bool {
        self.word
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |word| {
                (decode(word) == from).then_some(word & PINS | encode(to))
            })
            .is_ok()
    }
}

static ENTRIES: [Entry; CAPACITY] = [const { Entry::free() }; CAPACITY];

const fn encode(state: WorldState) -> u64 {
    let bits = match state {
        WorldState::Free => 0,
        WorldState::Live => 1,
        WorldState::Dying => 2,
        WorldState::Closed => 3,
    };
    bits << STATE_SHIFT
}

const fn decode(word: u64) -> WorldState {
    match (word >> STATE_SHIFT) & 0b11 {
        0 => WorldState::Free,
        1 => WorldState::Live,
        2 => WorldState::Dying,
        _ => WorldState::Closed,
    }
}

fn entry(node: NodeRef) -> Option<&'static Entry> {
    ENTRIES.get(node.node as usize)
}

fn entry_of(arena: ArenaId) -> &'static Entry {
    let Some(entry) = ENTRIES.get(arena.index() as usize) else {
        panic!("arena node {} lies outside the registry", arena.index());
    };
    entry
}

// The record lands in the world's own arena while the entry stays Free, so nothing resolves it
// before `publish`.
pub(super) fn prepare(
    context: &mut ThreadContext,
    arena: ArenaId,
    key: &WorldKey,
    path: Path,
    mask: ColourMask,
    limits: Limits,
    time: Time,
) -> Result<NodeRef, Refused> {
    let entry = entry_of(arena);
    assert!(
        entry.state().after(WorldEvent::Publish).is_ok(),
        "arena node {} was prepared while its world is not free",
        arena.index()
    );
    let mut earmark = Pool::<World, 1>::earmark(context, arena, 1)?;
    let mut page = pool::earmark(context, arena, 2)?;
    let (Some(frame), Some(home)) = (
        page.take(FrameKind::ArenaData),
        page.take(FrameKind::ArenaObject),
    ) else {
        return Err(Refused::NoMemory);
    };
    entry
        .clock
        .start(time.precision_ns, frame, home)
        .map_err(|_| Refused::NoMemory)?;
    let record = entry.records.insert(&mut earmark, || {
        World::new(arena, key, time.counter, path, mask, limits)
    });
    entry.record.store(record.to_bits(), Ordering::Relaxed);
    *entry.remains.lock_leaf() = Some(Remains {
        path,
        arena,
        killers: [None; KILLERS],
    });
    entry.generation.store(arena.generation(), Ordering::SeqCst);
    Ok(NodeRef {
        node: arena.index(),
        generation: arena.generation(),
    })
}

// Under the world's own life lock, which a dying ancestor's kill takes after it turned Dying, so
// either that kill finds it Live or this finds that ancestor not Live and refuses.
#[must_use = "a refused publish leaves the child for its creation to unwind"]
pub(crate) fn publish(node: NodeRef) -> bool {
    let Some(entry) = entry(node).filter(|entry| entry.is(node)) else {
        panic!("world {} was published after its node was reused", node.node);
    };
    let Some(world) = entry.records.hold(entry.record()) else {
        panic!("unpublished world {} lost its record", node.node);
    };
    let _life = world.life.lock_leaf();
    let ancestors_live = (0..world.path.depth()).all(|distance| {
        world
            .path
            .nearest(distance)
            .and_then(state)
            .is_some_and(|ancestor| ancestor == WorldState::Live)
    });
    if !ancestors_live {
        return false;
    }
    let published = entry.turn(WorldState::Free, WorldState::Live);
    assert!(published, "world {} was published twice", node.node);
    true
}

// Read under the world's life lock by a dying ancestor's kill, so a publish after it sees that
// ancestor Dying; `false` when the world is published or gone, for the caller to kill.
pub(super) fn unpublished(node: NodeRef) -> bool {
    let Some(entry) = entry(node) else {
        return false;
    };
    let Some(world) = entry
        .records
        .hold(entry.record())
        .filter(|world| world.node() == node)
    else {
        return false;
    };
    let _life = world.life.lock_leaf();
    entry.state() == WorldState::Free
}

// The record of a world that was prepared and never published, with its record pool closed;
// `None` while a dying ancestor's teardown still holds it.
pub(crate) fn withdraw(node: NodeRef) -> Option<World> {
    let Some(entry) =
        entry(node).filter(|entry| entry.is(node) && entry.state() == WorldState::Free)
    else {
        panic!("world {} was withdrawn though it was published", node.node);
    };
    match entry.records.destroy(entry.record()) {
        Ok(Some(world)) => {
            entry.records.close();
            entry.clock.stop();
            Some(world)
        }
        Err(Removal::Borrowed) => None,
        Ok(None) | Err(Removal::Refused(_)) => {
            panic!(
                "the record of unpublished world {} has another holder",
                node.node
            )
        }
    }
}

// The clock of a world for as long as its node keeps this generation.
pub(crate) fn clock(node: NodeRef) -> Option<&'static WorldClock> {
    entry(node)
        .filter(|entry| entry.is(node))
        .map(|entry| &entry.clock)
}

// The world's own record while it is Live or Dying, checked again once held since nodes are reused.
pub(crate) fn hold(node: NodeRef) -> Option<Hold<World>> {
    let entry = entry(node)?;
    if !entry.state().admits(Admission::Teardown) || !entry.is(node) {
        return None;
    }
    let world = entry.records.hold(entry.record())?;
    (world.node() == node).then_some(world)
}

pub(crate) fn state(node: NodeRef) -> Option<WorldState> {
    let entry = entry(node)?;
    let state = entry.state();
    entry.is(node).then_some(state)
}

// Under the world's life lock, so no descendant joins it after this.
pub(super) fn mark_dying(node: NodeRef) -> bool {
    entry(node)
        .is_some_and(|entry| entry.is(node) && entry.turn(WorldState::Live, WorldState::Dying))
}

// Closes the world to new pins and answers how many are still counted.
pub(super) fn close(node: NodeRef) -> u64 {
    let Some(entry) = entry(node).filter(|entry| entry.is(node)) else {
        panic!(
            "world {} was closed after its node was reclaimed",
            node.node
        );
    };
    entry.turn(WorldState::Dying, WorldState::Closed);
    entry.word.load(Ordering::SeqCst) & PINS
}

// The record of a closed world once no pin or hold borrows it, with its record pool closed.
pub(super) fn take(node: NodeRef) -> Option<World> {
    let entry =
        entry(node).filter(|entry| entry.is(node) && entry.state() == WorldState::Closed)?;
    match entry.records.destroy(entry.record()) {
        Ok(Some(world)) => {
            entry.records.close();
            entry.clock.stop();
            Some(world)
        }
        Err(Removal::Borrowed) => None,
        Ok(None) | Err(Removal::Refused(_)) => {
            panic!(
                "the record of closed world {} has another holder",
                node.node
            )
        }
    }
}

// Under a pin of the world, so the teardown reads every killer when it reclaims the node.
pub(super) fn enlist(node: NodeRef, killer: Killer) -> bool {
    let Some(entry) = entry(node).filter(|entry| entry.is(node)) else {
        return false;
    };
    let mut killers = entry.killers.lock_leaf();
    let Some(free) = killers.iter_mut().find(|killer| killer.is_none()) else {
        return false;
    };
    *free = Some(killer);
    true
}

// The next generation of the node starts Free; a node at the last generation stays Closed.
pub(super) fn reclaim(node: NodeRef) {
    let Some(entry) = entry(node).filter(|entry| entry.is(node)) else {
        panic!("world {} was reclaimed twice", node.node);
    };
    let Ok(reclaimed) = entry.state().reclaim(node.generation) else {
        panic!("world {} was reclaimed before it closed", node.node);
    };
    let killers = core::mem::replace(&mut *entry.killers.lock_leaf(), [None; KILLERS]);
    if let Some(remains) = entry.remains.lock_leaf().as_mut() {
        remains.killers = killers;
    }
    if let Reclaimed::Free { generation } = reclaimed {
        entry.generation.store(generation, Ordering::SeqCst);
        entry.turn(WorldState::Closed, WorldState::Free);
    }
}

pub(super) fn remains(node: NodeRef) -> Option<Remains> {
    *entry(node)?.remains.lock_leaf()
}

// A counted pin of one world for one bounded step, interrupts off; `None` answers `DEAD`.
pub(crate) struct Pin {
    world: Hold<World>,
    entry: &'static Entry,
    _guard: PinGuard<rank::Pin>,
}

pub(crate) fn pin(node: NodeRef, admission: Admission) -> Option<Pin> {
    let guard = PinGuard::take();
    let entry = entry(node)?;
    if !entry.is(node) {
        return None;
    }
    entry
        .word
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |word| {
            decode(word).admits(admission).then_some(word + 1)
        })
        .ok()?;
    let unpin = || entry.word.fetch_sub(1, Ordering::SeqCst);
    if !entry.is(node) {
        unpin();
        return None;
    }
    let Some(world) = entry.records.hold(entry.record()) else {
        unpin();
        return None;
    };
    Some(Pin {
        world,
        entry,
        _guard: guard,
    })
}

impl Pin {
    pub(crate) fn state(&self) -> WorldState {
        self.entry.state()
    }
}

impl Deref for Pin {
    type Target = World;

    fn deref(&self) -> &World {
        &self.world
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        self.entry.word.fetch_sub(1, Ordering::SeqCst);
    }
}

impl core::fmt::Debug for Pin {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Pin")
            .field("world", &*self.world)
            .finish()
    }
}
