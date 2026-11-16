use vstd::prelude::{verus, verus_spec, verus_verify};

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapperState {
    Free,
    Reserved,
    Occupied,
    Revoking,
    Retired,
}

// Commit, Abort and Unmap come from the mapping world; Revoke and Finish from the owner.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapperEvent {
    Claim,
    Commit(u32),
    Abort(u32),
    Unmap(u32),
    Revoke(u32),
    Finish(u32),
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MapperSlot {
    pub(crate) state: MapperState,
    pub(crate) generation: u32,
}

verus! {

impl MapperSlot {
    pub exec const FREE: Self = Self {
        state: MapperState::Free,
        generation: 0,
    };
}

} // verus!

#[verus_verify]
impl MapperSlot {
    #[verus_spec(r =>
        ensures
            r == self.spec_state(),
    )]
    pub fn state(self) -> MapperState {
        self.state
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_generation(),
    )]
    pub fn generation(self) -> u32 {
        self.generation
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(after) => crate::proof::mapper_listed(self, event) == Some(after),
                Err(MapperRefusal::Stale) => !crate::proof::mapper_names(self, event),
                Err(MapperRefusal::Revoking) => crate::proof::mapper_names(self, event)
                    && crate::proof::mapper_revoking_refuses(self, event),
                Err(MapperRefusal::Unexpected) => crate::proof::mapper_names(self, event)
                    && crate::proof::mapper_listed(self, event) is None
                    && !crate::proof::mapper_revoking_refuses(self, event),
            },
    )]
    pub fn next(self, event: MapperEvent) -> Result<Self, MapperRefusal> {
        let held = match event {
            MapperEvent::Claim => self.generation,
            MapperEvent::Commit(generation)
            | MapperEvent::Abort(generation)
            | MapperEvent::Unmap(generation)
            | MapperEvent::Revoke(generation)
            | MapperEvent::Finish(generation) => generation,
        };
        if held != self.generation {
            return Err(MapperRefusal::Stale);
        }
        match (self.state, event) {
            (MapperState::Free, MapperEvent::Claim) => Ok(self.with(MapperState::Reserved)),
            (MapperState::Reserved, MapperEvent::Commit(_)) => Ok(self.with(MapperState::Occupied)),
            (MapperState::Reserved | MapperState::Occupied, MapperEvent::Revoke(_)) => {
                Ok(self.with(MapperState::Revoking))
            }
            (MapperState::Reserved, MapperEvent::Abort(_))
            | (MapperState::Occupied, MapperEvent::Unmap(_))
            | (MapperState::Revoking, MapperEvent::Finish(_)) => Ok(self.freed()),
            (
                MapperState::Revoking,
                MapperEvent::Commit(_) | MapperEvent::Abort(_) | MapperEvent::Unmap(_),
            ) => Err(MapperRefusal::Revoking),
            _ => Err(MapperRefusal::Unexpected),
        }
    }

    #[verus_spec(r =>
        ensures
            r == crate::proof::mapper_with(self, state),
    )]
    fn with(self, state: MapperState) -> Self {
        Self { state, ..self }
    }

    #[verus_spec(r =>
        ensures
            r == crate::proof::mapper_freed(self),
    )]
    fn freed(self) -> Self {
        match self.generation.checked_add(1) {
            Some(generation) => Self {
                state: MapperState::Free,
                generation,
            },
            None => self.with(MapperState::Retired),
        }
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapperRefusal {
    Stale,
    Revoking,
    Unexpected,
}
