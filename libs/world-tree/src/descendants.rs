use core::fmt;

use crate::{NodeRef, UNUSED, WorldState};

// A set held in insertion order up to `count`; positions past it hold `UNUSED`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Descendants<const N: usize> {
    entries: [NodeRef; N],
    count: u32,
}

impl<const N: usize> Descendants<N> {
    pub const EMPTY: Self = Self {
        entries: [UNUSED; N],
        count: 0,
    };

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn get(&self, position: u32) -> Option<NodeRef> {
        let at = position as usize;
        if position < self.count && at < N {
            Some(self.entries[at])
        } else {
            None
        }
    }

    pub fn contains(&self, node: NodeRef) -> bool {
        self.position(node).is_some()
    }

    pub fn insert(&mut self, owner: WorldState, node: NodeRef) -> Result<(), DescendantRefusal> {
        if owner != WorldState::Live {
            return Err(DescendantRefusal::Frozen);
        }
        if self.contains(node) {
            return Err(DescendantRefusal::Present);
        }
        let at = self.count as usize;
        if at >= N || self.count == u32::MAX {
            return Err(DescendantRefusal::Full);
        }
        self.entries[at] = node;
        self.count += 1;
        Ok(())
    }

    pub fn remove(&mut self, node: NodeRef) -> Result<(), DescendantRefusal> {
        let Some(at) = self.position(node) else {
            return Err(DescendantRefusal::Absent);
        };
        let last = (self.count - 1) as usize;
        if last >= N {
            return Err(DescendantRefusal::Absent);
        }
        self.entries[at] = self.entries[last];
        self.entries[last] = UNUSED;
        self.count -= 1;
        Ok(())
    }

    fn position(&self, node: NodeRef) -> Option<usize> {
        let mut at = 0;
        while at < self.count as usize && at < N {
            if self.entries[at] == node {
                return Some(at);
            }
            at += 1;
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DescendantRefusal {
    Frozen,
    Present,
    Full,
    Absent,
}

impl fmt::Display for DescendantRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Frozen => "the owning world is not live, so its descendant set is frozen",
            Self::Present => "the world is already a descendant",
            Self::Full => "the descendant set is at capacity",
            Self::Absent => "the world is not a descendant",
        })
    }
}

impl core::error::Error for DescendantRefusal {}
