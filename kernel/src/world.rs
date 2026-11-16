pub(crate) mod registry;

use core::fmt;

use entropy::NotReady;
use world_tree::NodeRef;
use wost::mm::pool::{self, ArenaId, Refused};

use crate::thread::ThreadContext;
use crate::{info, random, warn};

// The kernel keeps 1/32 of each colour for its heap, page tables and threads.
const MARGIN_DIVISOR: u32 = 32;

pub(crate) struct World {
    arena: ArenaId,
    #[expect(
        dead_code,
        reason = "the values derived from it come with the handle tables"
    )]
    key: [u8; 32],
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
    registry::publish(context, arena, World { arena, key }).map_err(RootRefusal::Arena)
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
