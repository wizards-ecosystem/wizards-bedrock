mod create;
pub(crate) mod registry;
mod teardown;

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use entropy::{Local, NotReady};
use handle_space::Mixer;
use world_tree::{Budgets, Descendants, Limits, NodeRef, Path, WorldState};
use wost::arch::current::cpu::CounterAccess;
use wost::mm;
use wost::mm::pool::{self, ArenaId, ColourMask, Refused};
use wost::sync::{Leaf, SpinLock};

use crate::arena::Pool;
use crate::channel::Endpoint;
use crate::handle::Table;
use crate::memory::{CodeQuota, MemoryObject};
use crate::process::{Observation, Process};
use crate::queue::Queue;
use crate::thread::ThreadContext;
use crate::{info, random, rank, thread, timepage, warn};

pub(crate) use create::{Refusal, Request, create};
pub(crate) use teardown::{KillRefusal, inspect, kill, kill_for, step};

// The kernel keeps 1/32 of each colour for its heap, page tables and threads.
const MARGIN_DIVISOR: u32 = 32;

const ROOT_HANDLES: u32 = 64;

const CODE_PAGES: u32 = 65536;

// Every world but the root can be a descendant of the root, so this bounds the worlds alive at once.
const DESCENDANTS: usize = 63;

struct WorldKey([u8; 32]);

impl Drop for WorldKey {
    fn drop(&mut self) {
        mm::wipe(&mut self.0);
    }
}

// A `WorldKill` call that completes once the world's frames are credited to its heir.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Killer {
    pub(crate) queue: crate::queue::QueueRef,
    pub(crate) pending: usize,
    pub(crate) id: u64,
}

// A world's clock precision and its access to the counter, fixed when the world is made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Time {
    pub(crate) precision_ns: u64,
    pub(crate) counter: CounterAccess,
}

// Children join and leave `descendants` and debit `budgets` only under this lock.
#[derive(Debug)]
struct Life {
    budgets: Budgets,
    descendants: Descendants<DESCENDANTS>,
}

pub(crate) struct World {
    arena: ArenaId,
    key: WorldKey,
    // Keyed from the kernel generator, never from `key`, so the stored key reveals no output.
    stream: SpinLock<rank::Object, Local>,
    counter: CounterAccess,
    path: Path,
    mask: ColourMask,
    limits: Limits,
    life: SpinLock<rank::Object, Life>,
    ordinals: AtomicU64,
    spaces: AtomicU64,
    // Endpoints that lost their last reference under a pin, for the reaper to destroy.
    pub(crate) condemned: AtomicU64,
    pub(crate) code: CodeQuota,
    pub(crate) tables: TablePool,
    pub(crate) queues: QueuePool,
    pub(crate) endpoints: EndpointPool,
    pub(crate) processes: ProcessPool,
    pub(crate) observations: ObservationPool,
    pub(crate) memories: MemoryPool,
}

pub(crate) type TablePool = Pool<Table, 16>;
pub(crate) type QueuePool = Pool<Queue, 2>;
pub(crate) type EndpointPool = Pool<Endpoint, 4>;
pub(crate) type ProcessPool = Pool<Process, 4>;
pub(crate) type ObservationPool = Pool<Observation, 16>;
pub(crate) type MemoryPool = Pool<MemoryObject, 16>;

impl World {
    fn new(
        arena: ArenaId,
        key: &WorldKey,
        counter: CounterAccess,
        path: Path,
        mask: ColourMask,
        limits: Limits,
    ) -> Self {
        Self {
            arena,
            key: WorldKey(key.0),
            stream: SpinLock::new(Local::new(mm::wipe)),
            counter,
            path,
            mask,
            limits,
            life: SpinLock::new(Life {
                budgets: Budgets::new(limits),
                descendants: Descendants::EMPTY,
            }),
            ordinals: AtomicU64::new(0),
            spaces: AtomicU64::new(0),
            condemned: AtomicU64::new(crate::channel::NO_ENDPOINT),
            code: CodeQuota::new(CODE_PAGES),
            tables: Pool::new(),
            queues: Pool::new(),
            endpoints: Pool::new(),
            processes: Pool::new(),
            observations: Pool::new(),
            memories: Pool::new(),
        }
    }

    pub(crate) const fn arena(&self) -> ArenaId {
        self.arena
    }

    pub(crate) const fn node(&self) -> NodeRef {
        NodeRef {
            node: self.arena.index(),
            generation: self.arena.generation(),
        }
    }

    // This world or one of its descendants, which a handle into this world may name.
    pub(crate) fn governs(&self, node: NodeRef) -> bool {
        node == self.node() || self.life.lock_leaf().descendants.contains(node)
    }

    pub(crate) const fn handles(&self) -> u32 {
        self.limits.handles
    }

    pub(crate) const fn counter(&self) -> CounterAccess {
        self.counter
    }

    // The key that made each output is replaced before the output returns.
    pub(crate) fn fill_random(
        &self,
        context: &mut ThreadContext,
        out: &mut [u8],
    ) -> Result<(), NotReady> {
        random::fill_stream(&mut self.stream.lock(context), out)
    }

    // Every object page goes back to the arena; a live object here outlived each of its holders.
    fn release(self) {
        self.tables.close();
        self.queues.close();
        self.endpoints.close();
        self.processes.close();
        self.observations.close();
        self.memories.close();
    }

    // Derived once per process from the world key and the process's ordinal, then stored.
    pub(crate) fn mixer(&self) -> Mixer {
        let ordinal = self.ordinals.fetch_add(1, Ordering::Relaxed);
        let [a, b, c, d, e, f, g, h, ..] =
            entropy::derive(&self.key.0, b"handle", ordinal, mm::wipe);
        Mixer::from_derived([a, b, c, d, e, f, g, h])
    }

    // Derived once per address space from the world key and the space's ordinal, then stored.
    pub(crate) fn placement_seed(&self) -> [u8; 32] {
        let ordinal = self.spaces.fetch_add(1, Ordering::Relaxed);
        entropy::derive(&self.key.0, b"space", ordinal, mm::wipe)
    }
}

static ROOT: SpinLock<Leaf, Option<NodeRef>> = SpinLock::new(None);

pub(crate) fn root() -> Option<NodeRef> {
    *ROOT.lock_leaf()
}

// A thread of a dying world never returns to ring 3, so its process ends killed.
pub(crate) fn current_is_dying() -> bool {
    thread::current_process()
        .is_some_and(|process| registry::state(process.world()) != Some(WorldState::Live))
}

// The key never reaches a log.
impl fmt::Debug for World {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("World")
            .field("arena", &self.arena)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RootRefusal {
    Randomness(NotReady),
    Arena(Refused),
}

impl fmt::Display for RootRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Randomness(reason) => write!(formatter, "no world key: {reason}"),
            Self::Arena(reason) => write!(formatter, "no root arena: {reason}"),
        }
    }
}

impl core::error::Error for RootRefusal {}

fn create_root(context: &mut ThreadContext) -> Result<NodeRef, RootRefusal> {
    let mut key = WorldKey([0; 32]);
    random::fill(&mut key.0).map_err(RootRefusal::Randomness)?;
    let arena = pool::reserve_root(context, MARGIN_DIVISOR).map_err(RootRefusal::Arena)?;
    let limits = Limits {
        descendants: DESCENDANTS as u32,
        threads: thread::CAPACITY as u32,
        handles: ROOT_HANDLES,
    };
    // Only the root is given the raw time right; every other world inherits it from a holder.
    let time = Time {
        precision_ns: timepage::PRECISION_NS,
        counter: CounterAccess::Raw,
    };
    let root = registry::prepare(
        context,
        arena,
        &key,
        Path::ROOT,
        pool::full_mask(),
        limits,
        time,
    )
    .map_err(RootRefusal::Arena)?;
    registry::publish(root);
    *ROOT.lock_leaf() = Some(root);
    Ok(root)
}

pub(crate) fn start_root(context: &mut ThreadContext) {
    let root = match create_root(context) {
        Ok(root) => root,
        Err(reason) => {
            warn!("world: no root world: {reason}");
            return;
        }
    };
    let Some(usage) = registry::hold(root).and_then(|record| pool::usage(record.arena)) else {
        panic!(
            "the root world {} did not resolve after its publication",
            root.node
        );
    };
    info!(
        "world: root world {} reserves {} frames, its record and time page use {}",
        root.node,
        usage.reserved(),
        usage.used()
    );
    info!(
        "time: world clock page v{}, world time from 0 in steps of {} ns",
        native_abi::timepage::VERSION,
        timepage::PRECISION_NS
    );
}
