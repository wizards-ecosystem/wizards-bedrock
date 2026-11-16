use vstd::prelude::{verus_spec, verus_verify};

use crate::Unexpected;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Exited(u64),
    Faulted(u8),
    Killed,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    Running,
    Exited(Status),
    Released,
}

#[verus_verify]
impl ProcessState {
    // The first exit wins; a later exit, fault or kill is refused and the status stays.
    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => crate::proof::running_then_exited_then_released(self, next)
                    && next == ProcessState::Exited(status),
                Err(_) => self != ProcessState::Running,
            },
    )]
    pub fn exit(self, status: Status) -> Result<Self, Unexpected> {
        match self {
            Self::Running => Ok(Self::Exited(status)),
            Self::Exited(_) | Self::Released => Err(Unexpected),
        }
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => crate::proof::running_then_exited_then_released(self, next),
                Err(_) => !(self is Exited),
            },
    )]
    pub fn release(self) -> Result<Self, Unexpected> {
        match self {
            Self::Exited(_) => Ok(Self::Released),
            Self::Running | Self::Released => Err(Unexpected),
        }
    }

    #[verus_spec(r =>
        ensures
            r == match self {
                ProcessState::Exited(status) => Some(status),
                _ => None,
            },
    )]
    pub fn status(self) -> Option<Status> {
        match self {
            Self::Exited(status) => Some(status),
            Self::Running | Self::Released => None,
        }
    }
}
