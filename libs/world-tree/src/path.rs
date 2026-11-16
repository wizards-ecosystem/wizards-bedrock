use core::fmt;

use crate::{NodeRef, UNUSED, WorldState};

pub const MAX_DEPTH: u8 = 16;

const SLOTS: usize = MAX_DEPTH as usize;

// Ancestors root first; positions past `depth` hold `UNUSED`, so equal paths compare equal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Path {
    nodes: [NodeRef; SLOTS],
    depth: u8,
}

impl Path {
    pub const ROOT: Self = Self {
        nodes: [UNUSED; SLOTS],
        depth: 0,
    };

    pub fn depth(&self) -> u8 {
        self.depth
    }

    pub fn child(&self, parent: NodeRef) -> Result<Self, TooDeep> {
        let at = usize::from(self.depth);
        if at >= SLOTS {
            return Err(TooDeep);
        }
        let mut nodes = self.nodes;
        nodes[at] = parent;
        Ok(Self {
            nodes,
            depth: self.depth + 1,
        })
    }

    // Distance 0 is the parent.
    pub fn nearest(&self, distance: u8) -> Option<NodeRef> {
        if distance >= self.depth {
            return None;
        }
        let at = usize::from(self.depth - 1 - distance);
        if at < SLOTS {
            Some(self.nodes[at])
        } else {
            None
        }
    }

    pub fn contains(&self, node: NodeRef) -> bool {
        let mut at = 0;
        while at < usize::from(self.depth) && at < SLOTS {
            if self.nodes[at] == node {
                return true;
            }
            at += 1;
        }
        false
    }

    // `states[d]` is the state of the ancestor at distance `d`.
    pub fn heir(&self, states: &[WorldState; SLOTS]) -> Option<NodeRef> {
        let mut distance: u8 = 0;
        while distance < self.depth && usize::from(distance) < SLOTS {
            if states[usize::from(distance)] == WorldState::Live {
                return self.nearest(distance);
            }
            distance += 1;
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TooDeep;

impl fmt::Display for TooDeep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the parent already has the maximum number of ancestors")
    }
}

impl core::error::Error for TooDeep {}
