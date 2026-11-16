use core::fmt;

use crate::Unexpected;

const STATE_BITS: u32 = 4;
const STATE_MASK: u64 = (1 << STATE_BITS) - 1;

pub const GENERATION_LIMIT: u64 = 1 << (u64::BITS - STATE_BITS);

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

impl CallState {
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

    pub fn is_outcome(self) -> bool {
        matches!(
            self,
            Self::Replied | Self::Abandoned | Self::Cancelled | Self::TimedOut | Self::PeerClosed
        )
    }
}

// The value of the cell's `AtomicU64`: the state in bits 0 to 3, the call generation above.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    state: CallState,
    generation: u64,
}

impl Cell {
    pub const IDLE: Self = Self {
        state: CallState::Idle,
        generation: 0,
    };

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
        Some(Self {
            state,
            generation: bits >> STATE_BITS,
        })
    }

    pub fn bits(self) -> u64 {
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

    pub fn state(self) -> CallState {
        self.state
    }

    pub fn generation(self) -> u64 {
        self.generation
    }

    // `generation` is the call the event names; an event for an earlier call is refused.
    pub fn next(self, event: CallEvent, generation: u64) -> Result<Self, CellRefusal> {
        if generation != self.generation {
            return Err(CellRefusal::Stale);
        }
        let state = match self.state.next(event) {
            Ok(state) => state,
            Err(Unexpected) => return Err(CellRefusal::Unexpected),
        };
        if event != CallEvent::Consume {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellRefusal {
    Stale,
    Unexpected,
    Exhausted,
}

impl fmt::Display for CellRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stale => "the event names an earlier call",
            Self::Unexpected => "the event has no transition from the cell's state",
            Self::Exhausted => "the cell has used its last call generation",
        })
    }
}

impl core::error::Error for CellRefusal {}
