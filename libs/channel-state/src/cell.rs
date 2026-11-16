#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

use crate::Unexpected;

#[verus_verify]
const STATE_BITS: u32 = 4;
#[verus_verify]
const STATE_MASK: u64 = 0b1111;

#[verus_verify]
pub const GENERATION_LIMIT: u64 = 0x1000_0000_0000_0000;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CallState {
    Idle,
    Queued,
    Delivered,
    Replying,
    Replied,
    Abandoned,
    Cancelled,
    TimedOut,
    PeerClosed,
}

// Commit, Cancel, Timeout and Consume come from the caller; Deliver and Close from the endpoint;
// Claim, Complete and Abandon from the reply right.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CallEvent {
    Commit,
    Deliver,
    Claim,
    Complete,
    Abandon,
    Cancel,
    Timeout,
    Close,
    Consume,
}

#[verus_verify]
impl CallState {
    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => crate::proof::call_step(self, event) == Some(next),
                Err(_) => crate::proof::call_step(self, event) is None,
            },
    )]
    pub fn next(self, event: CallEvent) -> Result<Self, Unexpected> {
        match (self, event) {
            (Self::Idle, CallEvent::Commit) => Ok(Self::Queued),
            (Self::Queued, CallEvent::Deliver) => Ok(Self::Delivered),
            (Self::Queued, CallEvent::Close) => Ok(Self::PeerClosed),
            (Self::Queued | Self::Delivered, CallEvent::Cancel) => Ok(Self::Cancelled),
            (Self::Queued | Self::Delivered, CallEvent::Timeout) => Ok(Self::TimedOut),
            (Self::Delivered, CallEvent::Claim) => Ok(Self::Replying),
            (Self::Delivered, CallEvent::Abandon) => Ok(Self::Abandoned),
            (Self::Replying, CallEvent::Complete) => Ok(Self::Replied),
            (
                Self::Replied
                | Self::Abandoned
                | Self::Cancelled
                | Self::TimedOut
                | Self::PeerClosed,
                CallEvent::Consume,
            ) => Ok(Self::Idle),
            _ => Err(Unexpected),
        }
    }

    #[verus_spec(r =>
        ensures
            r == crate::proof::is_outcome(self),
    )]
    pub fn is_outcome(self) -> bool {
        matches!(
            self,
            Self::Replied | Self::Abandoned | Self::Cancelled | Self::TimedOut | Self::PeerClosed
        )
    }
}

// The value of the cell's `AtomicU64`: the state in bits 0 to 3, the call generation above.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub(crate) state: CallState,
    pub(crate) generation: u64,
}

verus! {

impl Cell {
    pub exec const IDLE: Self = Self {
        state: CallState::Idle,
        generation: 0,
    };
}

} // verus!

#[verus_verify]
impl Cell {
    #[verus_spec(r =>
        ensures
            match r {
                Some(cell) => cell.spec_bits() == bits,
                None => bits & 15 > 8,
            },
    )]
    pub fn from_bits(bits: u64) -> Option<Self> {
        let state = match bits & STATE_MASK {
            0 => CallState::Idle,
            1 => CallState::Queued,
            2 => CallState::Delivered,
            3 => CallState::Replying,
            4 => CallState::Replied,
            5 => CallState::Abandoned,
            6 => CallState::Cancelled,
            7 => CallState::TimedOut,
            8 => CallState::PeerClosed,
            _ => return None,
        };
        proof! {
            crate::proof::lemma_cell_unpacks(bits);
        }
        Some(Self {
            state,
            generation: bits >> STATE_BITS,
        })
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_bits(),
            forall|other: Cell|
                other.spec_generation() < GENERATION_LIMIT && #[trigger] other.spec_bits() == r
                    ==> other == self,
    )]
    pub fn bits(self) -> u64 {
        proof! {
            use_type_invariant(&self);
            crate::proof::lemma_cell_packs(self.generation, crate::proof::state_code(self.state));
            assert forall|other: Cell|
                other.spec_generation() < GENERATION_LIMIT && #[trigger] other.spec_bits()
                    == self.spec_bits() implies other == self by {
                crate::proof::lemma_cell_packs(
                    other.generation,
                    crate::proof::state_code(other.state),
                );
            }
        }
        let state: u64 = match self.state {
            CallState::Idle => 0,
            CallState::Queued => 1,
            CallState::Delivered => 2,
            CallState::Replying => 3,
            CallState::Replied => 4,
            CallState::Abandoned => 5,
            CallState::Cancelled => 6,
            CallState::TimedOut => 7,
            CallState::PeerClosed => 8,
        };
        (self.generation << STATE_BITS) | state
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_state(),
    )]
    pub fn state(self) -> CallState {
        self.state
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_generation(),
            r < GENERATION_LIMIT,
    )]
    pub fn generation(self) -> u64 {
        proof! {
            use_type_invariant(&self);
        }
        self.generation
    }

    // `generation` is the call the event names; an event for an earlier call is refused.
    #[verus_spec(r =>
        ensures
            match r {
                Ok(after) => crate::proof::cell_accepts(self, event, generation)
                    && after == crate::proof::cell_after(self, event, generation),
                Err(CellRefusal::Stale) => generation != self.spec_generation(),
                Err(CellRefusal::Unexpected) => generation == self.spec_generation()
                    && crate::proof::call_step(self.spec_state(), event) is None,
                Err(CellRefusal::Exhausted) => generation == self.spec_generation()
                    && event == CallEvent::Consume
                    && crate::proof::call_step(self.spec_state(), event) is Some
                    && self.spec_generation() + 1 == GENERATION_LIMIT,
            },
            r is Err ==> !crate::proof::cell_accepts(self, event, generation),
    )]
    pub fn next(self, event: CallEvent, generation: u64) -> Result<Self, CellRefusal> {
        proof! {
            use_type_invariant(&self);
        }
        if generation != self.generation {
            return Err(CellRefusal::Stale);
        }
        let state = match self.state.next(event) {
            Ok(state) => state,
            Err(Unexpected) => return Err(CellRefusal::Unexpected),
        };
        if !matches!(event, CallEvent::Consume) {
            return Ok(Self { state, ..self });
        }
        let next = self.generation + 1;
        if next >= GENERATION_LIMIT {
            return Err(CellRefusal::Exhausted);
        }
        Ok(Self {
            state,
            generation: next,
        })
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellRefusal {
    Stale,
    Unexpected,
    Exhausted,
}
