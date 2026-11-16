#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub const MAX_CPUS: u32 = 64;

pub const MAX_SEQUENCE: u64 = u64::MAX >> STATE_BITS;

const STATE_BITS: u32 = 3;

const STATE_MASK: u64 = (1 << STATE_BITS) - 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Running,
    Parking,
    Woken,
    Blocked,
    Waking,
    Dead,
}

impl State {
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
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Word(u64);

impl Word {
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn state(self) -> Option<State> {
        State::from_code(self.0 & STATE_MASK)
    }

    pub const fn sequence(self) -> u64 {
        self.0 >> STATE_BITS
    }

    const fn with(self, state: State) -> Self {
        Self((self.0 & !STATE_MASK) | state.code())
    }
}

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
pub const fn step(word: Word, event: Event) -> Option<Word> {
    let Some(state) = word.state() else {
        return None;
    };
    match (state, event) {
        (State::Running, Event::Park) => {
            if word.sequence() >= MAX_SEQUENCE {
                return None;
            }
            let parked = (word.sequence() + 1) << STATE_BITS;
            Some(Word(parked | State::Parking.code()))
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

const fn contains(set: u64, cpu: u32) -> bool {
    cpu < MAX_CPUS && set & (1 << cpu) != 0
}

const fn lowest(set: u64) -> Option<u32> {
    let mut cpu = 0;
    while cpu < MAX_CPUS {
        if set & (1 << cpu) != 0 {
            return Some(cpu);
        }
        cpu += 1;
    }
    None
}

#[must_use]
pub const fn wake_target(previous: u32, idle: u64, allowed: u64) -> Option<u32> {
    let previous_allowed = contains(allowed, previous);
    if previous_allowed && contains(idle, previous) {
        return Some(previous);
    }
    if let Some(cpu) = lowest(idle & allowed) {
        return Some(cpu);
    }
    if previous_allowed {
        return Some(previous);
    }
    lowest(allowed)
}

// `ready` counts each queue's threads that are queued and off every CPU.
#[must_use]
pub const fn steal_source(ready: &[u32], thief: usize) -> Option<usize> {
    let mut busiest = None;
    let mut most = 0;
    let mut index = 0;
    while index < ready.len() {
        if index != thief && ready[index] > most {
            most = ready[index];
            busiest = Some(index);
        }
        index += 1;
    }
    busiest
}

#[cfg(test)]
mod tests;
