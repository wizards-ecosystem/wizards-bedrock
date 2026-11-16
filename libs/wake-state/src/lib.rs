#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub const MAX_CPUS: u32 = 64;

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
    pub const fn word(self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Parking => 1,
            Self::Woken => 2,
            Self::Blocked => 3,
            Self::Waking => 4,
            Self::Dead => 5,
        }
    }

    pub const fn from_word(word: u8) -> Option<Self> {
        match word {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    Park,
    Block,
    Resume,
    Claim,
    Place,
    Exit,
}

// The caller moves its atomic from `word` to the returned state by compare-exchange.
#[must_use]
pub const fn step(word: u8, event: Event) -> Option<State> {
    let Some(state) = State::from_word(word) else {
        return None;
    };
    match (state, event) {
        (State::Running, Event::Park) => Some(State::Parking),
        (State::Parking, Event::Block) => Some(State::Blocked),
        (State::Woken, Event::Resume) | (State::Waking, Event::Place) => Some(State::Running),
        (State::Parking, Event::Claim) => Some(State::Woken),
        (State::Blocked, Event::Claim) => Some(State::Waking),
        (State::Running, Event::Exit) => Some(State::Dead),
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
