pub(crate) mod registry;

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use entropy::NotReady;
use handle_space::Mixer;
use world_tree::NodeRef;
use wost::mm::pool::{self, ArenaId, Refused};
use wost::sync::{Leaf, SpinLock};

use crate::arena::Pool;
use crate::handle::Table;
use crate::memory::MemoryObject;
use crate::object::Channel;
use crate::process::{Observation, Process};
use crate::queue::Queue;
use crate::thread::ThreadContext;
use crate::{info, random, warn};

// The kernel keeps 1/32 of each colour for its heap, page tables and threads.
const MARGIN_DIVISOR: u32 = 32;

const ROOT_HANDLES: u32 = 64;

pub(crate) struct World {
    arena: ArenaId,
    key: [u8; 32],
    handles: u32,
    ordinals: AtomicU64,
    spaces: AtomicU64,
    pub(crate) tables: TablePool,
    pub(crate) queues: QueuePool,
    pub(crate) channels: ChannelPool,
    pub(crate) processes: ProcessPool,
    pub(crate) observations: ObservationPool,
    pub(crate) memories: MemoryPool,
}

pub(crate) type TablePool = Pool<Table, 16>;
pub(crate) type QueuePool = Pool<Queue, 2>;
pub(crate) type ChannelPool = Pool<Channel, 64>;
pub(crate) type ProcessPool = Pool<Process, 4>;
pub(crate) type ObservationPool = Pool<Observation, 16>;
pub(crate) type MemoryPool = Pool<MemoryObject, 16>;

impl World {
    fn new(arena: ArenaId, key: [u8; 32], handles: u32) -> Self {
        Self {
            arena,
            key,
            handles,
            ordinals: AtomicU64::new(0),
            spaces: AtomicU64::new(0),
            tables: Pool::new(),
            queues: Pool::new(),
            channels: Pool::new(),
            processes: Pool::new(),
            observations: Pool::new(),
            memories: Pool::new(),
        }
    }

    pub(crate) const fn arena(&self) -> ArenaId {
        self.arena
    }

    pub(crate) const fn handles(&self) -> u32 {
        self.handles
    }

    // Derived once per process from the world key and the process's ordinal, then stored.
    pub(crate) fn mixer(&self) -> Mixer {
        let ordinal = self.ordinals.fetch_add(1, Ordering::Relaxed);
        let [a, b, c, d, e, f, g, h, ..] = entropy::derive(&self.key, b"handle", ordinal);
        Mixer::from_derived([a, b, c, d, e, f, g, h])
    }

    // Derived once per address space from the world key and the space's ordinal, then stored.
    pub(crate) fn placement_seed(&self) -> [u8; 32] {
        let ordinal = self.spaces.fetch_add(1, Ordering::Relaxed);
        entropy::derive(&self.key, b"space", ordinal)
    }
}

static ROOT: SpinLock<Leaf, Option<NodeRef>> = SpinLock::new(None);

pub(crate) fn root() -> Option<NodeRef> {
    *ROOT.lock_leaf()
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
    let mut key = [0; 32];
    random::fill(&mut key).map_err(RootRefusal::Randomness)?;
    let arena = pool::reserve_root(context, MARGIN_DIVISOR).map_err(RootRefusal::Arena)?;
    let world = World::new(arena, key, ROOT_HANDLES);
    let root = registry::publish(context, arena, world).map_err(RootRefusal::Arena)?;
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
        "world: root world {} reserves {} frames, its record uses {}",
        root.node,
        usage.reserved(),
        usage.used()
    );
}
