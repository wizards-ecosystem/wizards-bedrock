#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod proof;

#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

#[verus_verify]
pub const MAX_CPUS: u32 = 64;

#[verus_verify]
pub const MAX_SEQUENCE: u64 = u64::MAX >> 3;

#[verus_verify]
const STATE_BITS: u32 = 3;

#[verus_verify]
const STATE_MASK: u64 = 0b111;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Running,
    Parking,
    Woken,
    Blocked,
    Waking,
    Dead,
}

#[verus_verify]
impl State {
    #[verus_spec(r =>
        ensures
            r <= 5,
            proof::state_of_code(r) == Some(self),
    )]
    const fn code(self) -> u64 {
        match self {
            Self::Running => 0,
            Self::Parking => 1,
            Self::Woken => 2,
            Self::Blocked => 3,
            Self::Waking => 4,
            Self::Dead => 5,
        }
    }

    #[verus_spec(r =>
        ensures
            r == proof::state_of_code(code),
    )]
    const fn from_code(code: u64) -> Option<Self> {
        match code {
            0 => Some(Self::Running),
            1 => Some(Self::Parking),
            2 => Some(Self::Woken),
            3 => Some(Self::Blocked),
            4 => Some(Self::Waking),
            5 => Some(Self::Dead),
            _ => None,
        }
    }
}

// The state in the low bits and the sequence of the thread's latest wait above them.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Word(u64);

#[verus_verify]
impl Word {
    #[verus_spec(r =>
        ensures
            r.spec_bits() == bits,
    )]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_bits(),
    )]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_state(),
    )]
    pub const fn state(self) -> Option<State> {
        State::from_code(self.0 & STATE_MASK)
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_sequence(),
    )]
    pub const fn sequence(self) -> u64 {
        self.0 >> STATE_BITS
    }

    #[verus_spec(r =>
        ensures
            r.spec_state() == Some(state),
            r.spec_sequence() == self.spec_sequence(),
    )]
    const fn with(self, state: State) -> Self {
        let code = state.code();
        proof! {
            proof::lemma_with_keeps_the_sequence(self.0, code);
        }
        Self((self.0 & !STATE_MASK) | code)
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    Park,
    Block,
    Resume,
    Claim { sequence: u64 },
    Place,
    Exit,
}

// Applied by compare-exchange; a claim names its wait's sequence, so a late claim does nothing.
#[must_use]
#[verus_spec(r =>
    ensures
        match r {
            Some(next) => proof::wake_step(word, event) == proof::decoded(next),
            None => proof::wake_step(word, event) is None,
        },
)]
pub const fn step(word: Word, event: Event) -> Option<Word> {
    let Some(state) = word.state() else {
        return None;
    };
    match (state, event) {
        (State::Running, Event::Park) => {
            if word.sequence() >= MAX_SEQUENCE {
                return None;
            }
            let sequence = word.sequence() + 1;
            let code = State::Parking.code();
            proof! {
                proof::lemma_park_starts_a_sequence(sequence, code);
            }
            Some(Word((sequence << STATE_BITS) | code))
        }
        (State::Parking, Event::Block) => Some(word.with(State::Blocked)),
        (State::Woken, Event::Resume) | (State::Waking, Event::Place) => {
            Some(word.with(State::Running))
        }
        (State::Parking, Event::Claim { sequence }) if sequence == word.sequence() => {
            Some(word.with(State::Woken))
        }
        (State::Blocked, Event::Claim { sequence }) if sequence == word.sequence() => {
            Some(word.with(State::Waking))
        }
        (State::Running, Event::Exit) => Some(word.with(State::Dead)),
        _ => None,
    }
}

#[verus_spec(r =>
    ensures
        r == (cpu < 64 && set & (1u64 << cpu) != 0),
)]
const fn contains(set: u64, cpu: u32) -> bool {
    cpu < MAX_CPUS && set & (1 << cpu) != 0
}

verus! {

const fn lowest(set: u64) -> (r: Option<u32>)
    ensures
        match r {
            Some(cpu) => cpu < 64 && set & (1u64 << cpu) != 0,
            None => forall|cpu: u32| cpu < 64 ==> set & (1u64 << cpu) == 0,
        },
{
    let mut cpu = 0;
    while cpu < MAX_CPUS
        invariant
            cpu <= 64,
            forall|lower: u32| lower < cpu ==> set & (1u64 << lower) == 0,
        decreases 64 - cpu,
    {
        if set & (1 << cpu) != 0 {
            return Some(cpu);
        }
        cpu += 1;
    }
    None
}

} // verus!

#[must_use]
#[verus_spec(r =>
    ensures
        match r {
            Some(cpu) => cpu < 64 && allowed & (1u64 << cpu) != 0,
            None => forall|cpu: u32| cpu < 64 ==> allowed & (1u64 << cpu) == 0,
        },
)]
pub const fn wake_target(previous: u32, idle: u64, allowed: u64) -> Option<u32> {
    let previous_allowed = contains(allowed, previous);
    if previous_allowed && contains(idle, previous) {
        return Some(previous);
    }
    if let Some(cpu) = lowest(idle & allowed) {
        proof! {
            proof::lemma_idle_and_allowed_is_allowed(idle, allowed, cpu);
        }
        return Some(cpu);
    }
    if previous_allowed {
        return Some(previous);
    }
    lowest(allowed)
}

verus! {

// `ready` counts each queue's threads that are queued and off every CPU.
#[must_use]
pub const fn steal_source(ready: &[u32], thief: usize) -> (r: Option<usize>)
    ensures
        match r {
            Some(source) => {
                &&& source < ready@.len() && source != thief && ready@[source as int] > 0
                &&& forall|other: int|
                    0 <= other < ready@.len() && other != thief ==> ready@[other]
                        <= ready@[source as int]
            },
            None => forall|other: int|
                0 <= other < ready@.len() && other != thief ==> ready@[other] == 0,
        },
{
    let mut busiest: Option<usize> = None;
    let mut most = 0;
    let mut index = 0;
    while index < ready.len()
        invariant
            index <= ready@.len(),
            match busiest {
                Some(source) => source < index && source != thief && most == ready@[source as int]
                    && most > 0,
                None => most == 0,
            },
            forall|other: int| 0 <= other < index && other != thief ==> ready@[other] <= most,
        decreases ready@.len() - index,
    {
        if index != thief && ready[index] > most {
            most = ready[index];
            busiest = Some(index);
        }
        index += 1;
    }
    busiest
}

} // verus!

#[cfg(test)]
mod tests;
