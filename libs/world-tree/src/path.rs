#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

use crate::{NodeRef, UNUSED, WorldState};

#[verus_verify]
pub const MAX_DEPTH: u8 = 16;

#[verus_verify]
const SLOTS: usize = MAX_DEPTH as usize;

// Ancestors root first; positions past `depth` hold `UNUSED`, so equal paths compare equal.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Path {
    pub(crate) nodes: [NodeRef; SLOTS],
    pub(crate) depth: u8,
}

#[verus_verify]
impl Path {
    #[verus_spec(
        ensures
            Self::ROOT.ancestors().len() == 0,
    )]
    pub const ROOT: Self = Self {
        nodes: [UNUSED; SLOTS],
        depth: 0,
    };

    #[verus_spec(r =>
        ensures
            r as int == self.ancestors().len(),
            r <= 16,
    )]
    pub fn depth(&self) -> u8 {
        proof! {
            use_type_invariant(self);
        }
        self.depth
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(path) => path.ancestors() == self.ancestors().push(parent),
                Err(_) => self.ancestors().len() == 16,
            },
    )]
    pub fn child(&self, parent: NodeRef) -> Result<Self, TooDeep> {
        proof! {
            use_type_invariant(self);
        }
        let at = usize::from(self.depth);
        if at >= SLOTS {
            return Err(TooDeep);
        }
        let mut nodes = self.nodes;
        nodes[at] = parent;
        proof! {
            assert(nodes@.subrange(0, at + 1) =~= self.ancestors().push(parent));
        }
        Ok(Self {
            nodes,
            depth: self.depth + 1,
        })
    }

    // Distance 0 is the parent.
    #[verus_spec(r =>
        ensures
            r == self.at_distance(distance as int),
    )]
    pub fn nearest(&self, distance: u8) -> Option<NodeRef> {
        proof! {
            use_type_invariant(self);
        }
        if distance >= self.depth {
            return None;
        }
        Some(self.nodes[usize::from(self.depth - 1 - distance)])
    }
}

verus! {

impl Path {
    pub fn contains(&self, node: NodeRef) -> (r: bool)
        ensures
            r == self.ancestors().contains(node),
    {
        proof {
            use_type_invariant(self);
        }
        let depth = usize::from(self.depth);
        let mut at = 0;
        while at < depth
            invariant
                depth == self.depth,
                depth <= 16,
                at <= depth,
                forall|before: int| 0 <= before < at ==> self.nodes@[before] != node,
            decreases depth - at,
        {
            if self.nodes[at].same(node) {
                proof {
                    assert(self.ancestors()[at as int] == node);
                }
                return true;
            }
            at += 1;
        }
        proof {
            assert(forall|before: int|
                0 <= before < depth ==> self.ancestors()[before] == self.nodes@[before]);
        }
        false
    }

    // `states[d]` is the state of the ancestor at distance `d`.
    pub fn heir(&self, states: &[WorldState; SLOTS]) -> (r: Option<NodeRef>)
        ensures
            match r {
                Some(heir) => exists|distance: int|
                    crate::proof::heir_is_nearest_live_ancestor(*self, states@, distance)
                        && self.at_distance(distance) == Some(heir),
                None => forall|distance: int|
                    0 <= distance < self.ancestors().len() ==> states@[distance]
                        != WorldState::Live,
            },
    {
        proof {
            use_type_invariant(self);
        }
        let mut distance: u8 = 0;
        while distance < self.depth
            invariant
                distance <= self.depth <= 16,
                forall|nearer: int| 0 <= nearer < distance ==> states@[nearer] != WorldState::Live,
            decreases self.depth - distance,
        {
            if matches!(states[usize::from(distance)], WorldState::Live) {
                proof {
                    let nearest = distance as int;
                    assert(crate::proof::heir_is_nearest_live_ancestor(*self, states@, nearest));
                }
                return self.nearest(distance);
            }
            distance += 1;
        }
        None
    }

    // `states[d]` is the state of the ancestor at distance `d`, read under the child's life lock.
    pub fn publishes(&self, states: &[WorldState; SLOTS]) -> (r: bool)
        ensures
            crate::proof::published_only_under_live_ancestors(*self, states@, r),
    {
        proof {
            use_type_invariant(self);
        }
        let mut distance: u8 = 0;
        while distance < self.depth
            invariant
                distance <= self.depth <= 16,
                forall|nearer: int| 0 <= nearer < distance ==> states@[nearer] == WorldState::Live,
            decreases self.depth - distance,
        {
            if matches!(states[usize::from(distance)], WorldState::Live) {
                distance += 1;
            } else {
                proof {
                    assert(0 <= distance < self.ancestors().len());
                    assert(states@[distance as int] != WorldState::Live);
                }
                return false;
            }
        }
        true
    }
}

} // verus!

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TooDeep;
