#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{verus_spec, verus_verify};

use crate::Unexpected;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldState {
    Free,
    Live,
    Dying,
    Closed,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldEvent {
    Publish,
    Kill,
    Close,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Admission {
    Live,
    Teardown,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reclaimed {
    Free { generation: u32 },
    Retired,
}

#[verus_verify]
impl WorldState {
    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => crate::proof::live_then_dying_then_dead(self, event) == Some(next),
                Err(_) => crate::proof::live_then_dying_then_dead(self, event) is None,
            },
    )]
    pub fn after(self, event: WorldEvent) -> Result<Self, Unexpected> {
        match (self, event) {
            (Self::Free, WorldEvent::Publish) => Ok(Self::Live),
            (Self::Live, WorldEvent::Kill) => Ok(Self::Dying),
            (Self::Dying, WorldEvent::Close) => Ok(Self::Closed),
            _ => Err(Unexpected),
        }
    }

    #[verus_spec(r =>
        ensures
            r == (self == WorldState::Live
                || (self == WorldState::Dying && admission == Admission::Teardown)),
    )]
    pub fn admits(self, admission: Admission) -> bool {
        matches!(
            (self, admission),
            (Self::Live, _) | (Self::Dying, Admission::Teardown)
        )
    }

    // A node whose last generation died stays Closed, so no pin and no publish reaches it again.
    #[verus_spec(r =>
        ensures
            match r {
                Ok(Reclaimed::Free { generation: next }) => self == WorldState::Closed
                    && next == generation + 1,
                Ok(Reclaimed::Retired) => self == WorldState::Closed && generation == u32::MAX,
                Err(_) => self != WorldState::Closed,
            },
    )]
    pub fn reclaim(self, generation: u32) -> Result<Reclaimed, Unexpected> {
        if !matches!(self, Self::Closed) {
            return Err(Unexpected);
        }
        match generation.checked_add(1) {
            Some(generation) => Ok(Reclaimed::Free { generation }),
            None => Ok(Reclaimed::Retired),
        }
    }
}
