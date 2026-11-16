use crate::Unexpected;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Exited(u64),
    Faulted(u8),
    Killed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    Running,
    Exited(Status),
    Released,
}

impl ProcessState {
    // The first exit wins; a later exit, fault or kill is refused and the status stays.
    pub fn exit(self, status: Status) -> Result<Self, Unexpected> {
        match self {
            Self::Running => Ok(Self::Exited(status)),
            Self::Exited(_) | Self::Released => Err(Unexpected),
        }
    }

    pub fn release(self) -> Result<Self, Unexpected> {
        match self {
            Self::Exited(_) => Ok(Self::Released),
            Self::Running | Self::Released => Err(Unexpected),
        }
    }

    pub fn status(self) -> Option<Status> {
        match self {
            Self::Exited(status) => Some(status),
            Self::Running | Self::Released => None,
        }
    }
}
