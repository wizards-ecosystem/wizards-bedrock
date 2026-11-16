use crate::{Path, Unexpected};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CreationRefusal {
    Budget,
    NoMemory,
    Dead,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CreationEvent {
    Done,
    Pending,
    Halted(CreationRefusal),
}

// Each distance names an ancestor of the child: 0 is the parent.
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Creation {
    ancestors: u8,
    step: CreationStep,
}

impl Creation {
    pub fn new(child: &Path) -> Option<Self> {
        if child.depth() == 0 {
            return None;
        }
        Some(Self {
            ancestors: child.depth(),
            step: CreationStep::Debit,
        })
    }

    pub fn step(self) -> CreationStep {
        self.step
    }

    pub fn next(self, event: CreationEvent) -> Result<Self, Unexpected> {
        let step = match (self.step, event) {
            (CreationStep::Debit, CreationEvent::Done) => CreationStep::Build,
            (CreationStep::Debit, CreationEvent::Halted(cause)) => CreationStep::Refused(cause),
            (CreationStep::Build, CreationEvent::Done) => CreationStep::Insert(0),
            (CreationStep::Build, CreationEvent::Halted(cause)) => CreationStep::Credit(cause),
            (CreationStep::Insert(distance), CreationEvent::Done) => {
                let further = distance.saturating_add(1);
                if further < self.ancestors {
                    CreationStep::Insert(further)
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
                let further = distance.saturating_add(1);
                if further < self.ancestors {
                    CreationStep::Confirm(further)
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
