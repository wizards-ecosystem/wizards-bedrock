use crate::Unexpected;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldState {
    Free,
    Live,
    Dying,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldEvent {
    Publish,
    Kill,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Admission {
    Live,
    Teardown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reclaimed {
    Free { generation: u32 },
    Retired,
}

impl WorldState {
    pub fn after(self, event: WorldEvent) -> Result<Self, Unexpected> {
        match (self, event) {
            (Self::Free, WorldEvent::Publish) => Ok(Self::Live),
            (Self::Live, WorldEvent::Kill) => Ok(Self::Dying),
            (Self::Dying, WorldEvent::Close) => Ok(Self::Closed),
            _ => Err(Unexpected),
        }
    }

    pub fn admits(self, admission: Admission) -> bool {
        matches!(
            (self, admission),
            (Self::Live, _) | (Self::Dying, Admission::Teardown)
        )
    }

    // A node whose last generation died stays Closed, so no pin and no publish reaches it again.
    pub fn reclaim(self, generation: u32) -> Result<Reclaimed, Unexpected> {
        if self != Self::Closed {
            return Err(Unexpected);
        }
        match generation.checked_add(1) {
            Some(generation) => Ok(Reclaimed::Free { generation }),
            None => Ok(Reclaimed::Retired),
        }
    }
}
