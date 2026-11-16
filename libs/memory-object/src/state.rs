use core::fmt;

const STATE_BITS: u32 = 2;
const STATE_MASK: u64 = (1 << STATE_BITS) - 1;
const WORD_BITS: u32 = STATE_BITS + u32::BITS;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjectState {
    Writable,
    SealedRead,
    SealedExec,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seal {
    Read,
    Execute,
}

// The value of the object's state `AtomicU64`; each transition is applied by one compare and swap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StateWord {
    state: ObjectState,
    writers: u32,
}

impl StateWord {
    pub const WRITABLE: Self = Self {
        state: ObjectState::Writable,
        writers: 0,
    };

    pub fn from_bits(bits: u64) -> Option<Self> {
        if bits >> WORD_BITS != 0 {
            return None;
        }
        let state = match bits & STATE_MASK {
            0 => ObjectState::Writable,
            1 => ObjectState::SealedRead,
            2 => ObjectState::SealedExec,
            _ => return None,
        };
        Some(Self {
            state,
            writers: (bits >> STATE_BITS) as u32,
        })
    }

    pub fn bits(self) -> u64 {
        let state: u64 = match self.state {
            ObjectState::Writable => 0,
            ObjectState::SealedRead => 1,
            ObjectState::SealedExec => 2,
        };
        (u64::from(self.writers) << STATE_BITS) | state
    }

    pub fn state(self) -> ObjectState {
        self.state
    }

    pub fn writers(self) -> u32 {
        self.writers
    }

    pub fn acquire_writer(self) -> Result<Self, StateRefusal> {
        if self.state != ObjectState::Writable {
            return Err(StateRefusal::Sealed);
        }
        let Some(writers) = self.writers.checked_add(1) else {
            return Err(StateRefusal::Saturated);
        };
        Ok(Self { writers, ..self })
    }

    pub fn release_writer(self) -> Result<Self, StateRefusal> {
        if self.state != ObjectState::Writable || self.writers == 0 {
            return Err(StateRefusal::NoWriter);
        }
        Ok(Self {
            writers: self.writers - 1,
            ..self
        })
    }

    pub fn seal(self, seal: Seal) -> Result<Self, StateRefusal> {
        if self.state != ObjectState::Writable {
            return Err(StateRefusal::Sealed);
        }
        if self.writers != 0 {
            return Err(StateRefusal::Busy);
        }
        let state = match seal {
            Seal::Read => ObjectState::SealedRead,
            Seal::Execute => ObjectState::SealedExec,
        };
        Ok(Self { state, writers: 0 })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StateRefusal {
    Sealed,
    Busy,
    Saturated,
    NoWriter,
}

impl fmt::Display for StateRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sealed => "the object is sealed",
            Self::Busy => "the object still has a write reference",
            Self::Saturated => "the object's writer count is at its maximum",
            Self::NoWriter => "the object holds no write reference to release",
        })
    }
}

impl core::error::Error for StateRefusal {}
