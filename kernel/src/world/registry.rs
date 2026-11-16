use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use world_tree::{NodeRef, WorldEvent, WorldState};
use wost::mm::pool::{self, ArenaId, Refused};
use wost::mm::slab::Hold;

use super::World;
use crate::arena::{Local, Pool};
use crate::thread::ThreadContext;

const CAPACITY: usize = pool::capacity() as usize;

const STATE_SHIFT: u32 = 32;

// One entry per arena node; a world's record is the only object in its node's record pool.
#[derive(Debug)]
struct Entry {
    generation: AtomicU32,
    word: AtomicU64,
    record: AtomicU64,
    records: Pool<World, 1>,
}

impl Entry {
    const fn free() -> Self {
        Self {
            generation: AtomicU32::new(0),
            word: AtomicU64::new(encode(WorldState::Free)),
            record: AtomicU64::new(0),
            records: Pool::new(),
        }
    }

    fn state(&self) -> WorldState {
        decode(self.word.load(Ordering::Acquire))
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

// The record lands in the world's own arena before the entry turns Live, so a refusal leaves the
// entry Free.
pub(crate) fn publish(
    context: &mut ThreadContext,
    arena: ArenaId,
    world: World,
) -> Result<NodeRef, Refused> {
    let Some(entry) = ENTRIES.get(arena.index() as usize) else {
        panic!("arena node {} lies outside the registry", arena.index());
    };
    let Ok(live) = entry.state().after(WorldEvent::Publish) else {
        panic!(
            "arena node {} was published while its world is not free",
            arena.index()
        );
    };
    let mut earmark = Pool::<World, 1>::earmark(context, arena, 1)?;
    let record = entry.records.insert(&mut earmark, world);
    entry.record.store(record.to_bits(), Ordering::Relaxed);
    entry
        .generation
        .store(arena.generation(), Ordering::Relaxed);
    entry.word.store(encode(live), Ordering::Release);
    Ok(NodeRef {
        node: arena.index(),
        generation: arena.generation(),
    })
}

pub(crate) fn hold(node: NodeRef) -> Option<Hold<World>> {
    let entry = ENTRIES.get(node.node as usize)?;
    if entry.state() != WorldState::Live
        || entry.generation.load(Ordering::Relaxed) != node.generation
    {
        return None;
    }
    let record = Local::from_bits(entry.record.load(Ordering::Relaxed));
    entry.records.hold(record)
}
