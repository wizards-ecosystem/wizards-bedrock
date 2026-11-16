#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

#[verus_verify]
const STATE_BITS: u32 = 2;
#[verus_verify]
const STATE_MASK: u64 = 0b11;
#[verus_verify]
const WORD_BITS: u32 = STATE_BITS + u32::BITS;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjectState {
    Writable,
    SealedRead,
    SealedExec,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seal {
    Read,
    Execute,
}

// The value of the object's state `AtomicU64`; each transition is applied by one compare and swap.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StateWord {
    pub(crate) state: ObjectState,
    pub(crate) writers: u32,
}

verus! {

impl StateWord {
    pub exec const WRITABLE: Self = Self {
        state: ObjectState::Writable,
        writers: 0,
    };
}

} // verus!

#[verus_verify]
impl StateWord {
    #[verus_spec(r =>
        ensures
            match r {
                Some(word) => word.spec_bits() == bits,
                None => forall|word: StateWord| word.spec_bits() != bits,
            },
    )]
    pub fn from_bits(bits: u64) -> Option<Self> {
        proof! {
            assert forall|word: StateWord| word.spec_bits() == bits implies bits >> 34u32 == 0
                && bits & 3 <= 2 by {
                crate::proof::lemma_state_word_packs(
                    word.writers as u64,
                    crate::proof::state_code(word.state),
                );
            }
        }
        if bits >> WORD_BITS != 0 {
            return None;
        }
        let state = match bits & STATE_MASK {
            0 => ObjectState::Writable,
            1 => ObjectState::SealedRead,
            2 => ObjectState::SealedExec,
            _ => return None,
        };
        proof! {
            crate::proof::lemma_state_word_unpacks(bits);
        }
        Some(Self {
            state,
            writers: (bits >> STATE_BITS) as u32,
        })
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_bits(),
            forall|other: StateWord| other.spec_bits() == r ==> other == self,
    )]
    pub fn bits(self) -> u64 {
        proof! {
            crate::proof::lemma_state_word_packs(
                self.writers as u64,
                crate::proof::state_code(self.state),
            );
            assert forall|other: StateWord| other.spec_bits() == self.spec_bits() implies other
                == self by {
                crate::proof::lemma_state_word_packs(
                    other.writers as u64,
                    crate::proof::state_code(other.state),
                );
            }
        }
        let state: u64 = match self.state {
            ObjectState::Writable => 0,
            ObjectState::SealedRead => 1,
            ObjectState::SealedExec => 2,
        };
        (u64::from(self.writers) << STATE_BITS) | state
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_state(),
    )]
    pub fn state(self) -> ObjectState {
        self.state
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_writers(),
    )]
    pub fn writers(self) -> u32 {
        self.writers
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => self.spec_state() == ObjectState::Writable
                    && next.spec_writers() == self.spec_writers() + 1
                    && next.spec_state() == self.spec_state()
                    && crate::proof::state_moves_only_to_sealed(
                        self.spec_state(),
                        next.spec_state(),
                    ),
                Err(StateRefusal::Sealed) => self.spec_state() != ObjectState::Writable,
                Err(StateRefusal::Saturated) => self.spec_state() == ObjectState::Writable
                    && self.spec_writers() == u32::MAX,
                Err(_) => false,
            },
    )]
    pub fn acquire_writer(self) -> Result<Self, StateRefusal> {
        if !matches!(self.state, ObjectState::Writable) {
            return Err(StateRefusal::Sealed);
        }
        let Some(writers) = self.writers.checked_add(1) else {
            return Err(StateRefusal::Saturated);
        };
        Ok(Self { writers, ..self })
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => self.spec_state() == ObjectState::Writable
                    && self.spec_writers() > 0
                    && next.spec_writers() == self.spec_writers() - 1
                    && next.spec_state() == self.spec_state()
                    && crate::proof::state_moves_only_to_sealed(
                        self.spec_state(),
                        next.spec_state(),
                    ),
                Err(StateRefusal::NoWriter) => self.spec_state() != ObjectState::Writable
                    || self.spec_writers() == 0,
                Err(_) => false,
            },
    )]
    pub fn release_writer(self) -> Result<Self, StateRefusal> {
        if !matches!(self.state, ObjectState::Writable) || self.writers == 0 {
            return Err(StateRefusal::NoWriter);
        }
        Ok(Self {
            writers: self.writers - 1,
            ..self
        })
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => crate::proof::seal_admitted(self)
                    && next.spec_state() == crate::proof::sealed_as(seal)
                    && next.spec_writers() == 0
                    && crate::proof::state_moves_only_to_sealed(
                        self.spec_state(),
                        next.spec_state(),
                    ),
                Err(StateRefusal::Sealed) => self.spec_state() != ObjectState::Writable,
                Err(StateRefusal::Busy) => self.spec_state() == ObjectState::Writable
                    && self.spec_writers() != 0,
                Err(_) => false,
            },
    )]
    pub fn seal(self, seal: Seal) -> Result<Self, StateRefusal> {
        if !matches!(self.state, ObjectState::Writable) {
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

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StateRefusal {
    Sealed,
    Busy,
    Saturated,
    NoWriter,
}
