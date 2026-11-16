#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus_spec, verus_verify};

use crate::{Path, Unexpected};

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CreationRefusal {
    Budget,
    NoMemory,
    Dead,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CreationEvent {
    Done,
    Pending,
    Halted(CreationRefusal),
}

// Each distance names an ancestor of the child: 0 is the parent.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CreationStep {
    Debit,
    Build,
    Insert(u8),
    Publish,
    Confirm(u8),
    Remove(u8, CreationRefusal),
    Credit(CreationRefusal),
    Dissolve(CreationRefusal),
    Refused(CreationRefusal),
    Unwound(CreationRefusal),
    Published,
    Kill,
}

// An ancestor found Dying after the publish skipped the child in its kill, so the creator kills it.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Creation {
    pub(crate) ancestors: u8,
    pub(crate) step: CreationStep,
}

#[verus_verify]
impl Creation {
    #[verus_spec(r =>
        ensures
            match r {
                Some(creation) => child.ancestors().len() > 0
                    && creation.spec_ancestors() == child.ancestors().len()
                    && creation.spec_step() == CreationStep::Debit,
                None => child.ancestors().len() == 0,
            },
    )]
    pub fn new(child: &Path) -> Option<Self> {
        let ancestors = child.depth();
        if ancestors == 0 {
            return None;
        }
        Some(Self {
            ancestors,
            step: CreationStep::Debit,
        })
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_step(),
            crate::proof::names_an_ancestor(r, self.spec_ancestors()),
    )]
    pub fn step(self) -> CreationStep {
        proof! {
            use_type_invariant(self);
        }
        self.step
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => crate::proof::enters_every_ancestor_or_none(self, event, next)
                    && crate::proof::credits_what_it_debited(self, event, next)
                    && crate::proof::published_after_every_confirm(self, event, next),
                Err(_) => true,
            },
    )]
    pub fn next(self, event: CreationEvent) -> Result<Self, Unexpected> {
        proof! {
            use_type_invariant(self);
        }
        let step = match (self.step, event) {
            (CreationStep::Debit, CreationEvent::Done) => CreationStep::Build,
            (CreationStep::Debit, CreationEvent::Halted(cause)) => CreationStep::Refused(cause),
            (CreationStep::Build, CreationEvent::Done) => CreationStep::Insert(0),
            (CreationStep::Build, CreationEvent::Halted(cause)) => CreationStep::Credit(cause),
            (CreationStep::Insert(distance), CreationEvent::Done) => {
                if distance + 1 < self.ancestors {
                    CreationStep::Insert(distance + 1)
                } else {
                    CreationStep::Publish
                }
            }
            (CreationStep::Insert(distance), CreationEvent::Halted(cause))
            | (CreationStep::Remove(distance, cause), CreationEvent::Done) => {
                if distance == 0 {
                    CreationStep::Credit(cause)
                } else {
                    CreationStep::Remove(distance - 1, cause)
                }
            }
            (CreationStep::Credit(cause), CreationEvent::Done)
            | (CreationStep::Dissolve(cause), CreationEvent::Pending) => {
                CreationStep::Dissolve(cause)
            }
            (CreationStep::Dissolve(cause), CreationEvent::Done) => CreationStep::Unwound(cause),
            (CreationStep::Publish, CreationEvent::Done) => CreationStep::Confirm(0),
            (CreationStep::Confirm(distance), CreationEvent::Done) => {
                if distance + 1 < self.ancestors {
                    CreationStep::Confirm(distance + 1)
                } else {
                    CreationStep::Published
                }
            }
            (CreationStep::Confirm(_), CreationEvent::Halted(_)) => CreationStep::Kill,
            _ => return Err(Unexpected),
        };
        Ok(Self {
            ancestors: self.ancestors,
            step,
        })
    }
}
