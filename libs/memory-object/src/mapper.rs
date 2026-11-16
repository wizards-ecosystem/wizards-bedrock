use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapperState {
    Free,
    Reserved,
    Occupied,
    Revoking,
    Retired,
}

// Commit, Abort and Unmap come from the mapping world; Revoke and Finish from the owner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapperEvent {
    Claim,
    Commit(u32),
    Abort(u32),
    Unmap(u32),
    Revoke(u32),
    Finish(u32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MapperSlot {
    state: MapperState,
    generation: u32,
}

impl MapperSlot {
    pub const FREE: Self = Self {
        state: MapperState::Free,
        generation: 0,
    };

    pub fn state(self) -> MapperState {
        self.state
    }

    pub fn generation(self) -> u32 {
        self.generation
    }

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

    fn with(self, state: MapperState) -> Self {
        Self { state, ..self }
    }

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapperRefusal {
    Stale,
    Revoking,
    Unexpected,
}

impl fmt::Display for MapperRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stale => "the mapper slot has moved to another generation",
            Self::Revoking => "the owner is revoking the mapper slot",
            Self::Unexpected => "the event has no transition from this mapper slot state",
        })
    }
}

impl core::error::Error for MapperRefusal {}
