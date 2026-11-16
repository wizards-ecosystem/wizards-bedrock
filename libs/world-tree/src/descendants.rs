#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

use crate::{NodeRef, UNUSED, WorldState};

// A set held in insertion order up to `count`; positions past it hold `UNUSED`.
#[verus_verify]
#[derive(PartialEq, Eq, Debug)]
pub struct Descendants<const N: usize> {
    pub(crate) entries: [NodeRef; N],
    pub(crate) count: u32,
}

#[verus_verify]
impl<const N: usize> Descendants<N> {
    // An ensures makes the constant exec only; a generic constant cannot name itself in it.
    #[verus_spec(
        ensures
            true,
    )]
    pub const EMPTY: Self = Self {
        entries: [UNUSED; N],
        count: 0,
    };

    #[verus_spec(r =>
        ensures
            r as int == self.members().len(),
    )]
    pub fn count(&self) -> u32 {
        proof! {
            use_type_invariant(self);
        }
        self.count
    }

    #[verus_spec(r =>
        ensures
            r == if (position as int) < self.members().len() {
                Some(self.members()[position as int])
            } else {
                None
            },
    )]
    pub fn get(&self, position: u32) -> Option<NodeRef> {
        proof! {
            use_type_invariant(self);
        }
        if position < self.count {
            Some(self.entries[position as usize])
        } else {
            None
        }
    }

    #[verus_spec(r =>
        ensures
            r == self.members().contains(node),
    )]
    pub fn contains(&self, node: NodeRef) -> bool {
        let found = self.position(node);
        proof! {
            use_type_invariant(self);
            match found {
                Some(at) => assert(self.members()[at as int] == node),
                None => {},
            }
        }
        found.is_some()
    }

    #[verus_spec(r =>
        ensures
            crate::proof::nothing_joins_once_dying(owner, r),
            match r {
                Ok(()) => !old(self).members().contains(node)
                    && final(self).members() == old(self).members().push(node),
                Err(_) => *final(self) == *old(self),
            },
            old(self).tail_clear() ==> final(self).tail_clear(),
    )]
    pub fn insert(&mut self, owner: WorldState, node: NodeRef) -> Result<(), DescendantRefusal> {
        proof! {
            use_type_invariant(&*self);
        }
        if !matches!(owner, WorldState::Live) {
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
        proof! {
            assert forall|before: int| 0 <= before < at implies self.entries@[before] != node by {
                assert(old(self).members()[before] == self.entries@[before]);
            }
        }
        self.count += 1;
        proof! {
            assert(self.members() =~= old(self).members().push(node));
        }
        Ok(())
    }

    // The count drops first, so the set never holds the moved entry twice.
    #[verus_spec(r =>
        ensures
            match r {
                Ok(()) => old(self).members().contains(node)
                    && final(self).members().len() + 1 == old(self).members().len()
                    && forall|other: NodeRef| final(self).members().contains(other)
                        <==> (old(self).members().contains(other) && other != node),
                Err(_) => !old(self).members().contains(node) && *final(self) == *old(self),
            },
            old(self).tail_clear() ==> final(self).tail_clear(),
    )]
    pub fn remove(&mut self, node: NodeRef) -> Result<(), DescendantRefusal> {
        proof! {
            use_type_invariant(&*self);
        }
        let Some(at) = self.position(node) else {
            return Err(DescendantRefusal::Absent);
        };
        let last = (self.count - 1) as usize;
        self.count -= 1;
        self.entries[at] = self.entries[last];
        self.entries[last] = UNUSED;
        proof! {
            let before = old(self).members();
            let after = self.members();
            assert(before[at as int] == node);
            assert forall|other: NodeRef| after.contains(other)
                <==> (before.contains(other) && other != node) by {
                if before.contains(other) && other != node {
                    let k = choose|k: int| 0 <= k < before.len() && before[k] == other;
                    if k == last {
                        assert(after[at as int] == other);
                    } else {
                        assert(after[k] == other);
                    }
                }
                if after.contains(other) {
                    let k = choose|k: int| 0 <= k < after.len() && after[k] == other;
                    if k == at {
                        assert(before[last as int] == other);
                    } else {
                        assert(before[k] == other);
                    }
                }
            }
        }
        Ok(())
    }
}

#[verus_verify]
impl<const N: usize> Clone for Descendants<N> {
    fn clone(&self) -> Self {
        proof! {
            use_type_invariant(self);
        }
        Self {
            entries: self.entries,
            count: self.count,
        }
    }
}

verus! {

impl<const N: usize> Descendants<N> {
    fn position(&self, node: NodeRef) -> (r: Option<usize>)
        ensures
            match r {
                Some(at) => at < self.count && self.entries@[at as int] == node,
                None => !self.members().contains(node),
            },
    {
        proof {
            use_type_invariant(self);
        }
        let count = self.count as usize;
        let mut at = 0;
        while at < count
            invariant
                count == self.count,
                count <= N,
                at <= count,
                forall|before: int| 0 <= before < at ==> self.entries@[before] != node,
            decreases count - at,
        {
            if self.entries[at].same(node) {
                return Some(at);
            }
            at += 1;
        }
        proof {
            assert(forall|before: int|
                0 <= before < count ==> self.members()[before] == self.entries@[before]);
        }
        None
    }
}

} // verus!

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DescendantRefusal {
    Frozen,
    Present,
    Full,
    Absent,
}
