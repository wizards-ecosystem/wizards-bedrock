use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

use crate::Word;
#[cfg(verus_only)]
use crate::{Event, State};

verus! {

impl Word {
    pub open(crate) spec fn spec_bits(self) -> u64 {
        self.0
    }

    pub open(crate) spec fn spec_state(self) -> Option<State> {
        state_of_code(self.0 & crate::STATE_MASK)
    }

    pub open(crate) spec fn spec_sequence(self) -> u64 {
        self.0 >> crate::STATE_BITS
    }
}

pub open spec fn state_of_code(code: u64) -> Option<State> {
    match code {
        0 => Some(State::Running),
        1 => Some(State::Parking),
        2 => Some(State::Woken),
        3 => Some(State::Blocked),
        4 => Some(State::Waking),
        5 => Some(State::Dead),
        _ => None,
    }
}

pub open spec fn decoded(word: Word) -> Option<(State, u64)> {
    match word.spec_state() {
        Some(state) => Some((state, word.spec_sequence())),
        None => None,
    }
}

pub open spec fn wake_step(word: Word, event: Event) -> Option<(State, u64)> {
    match word.spec_state() {
        Some(state) => wake_transition(state, word.spec_sequence(), event),
        None => None,
    }
}

// The wake word's moves, as the TLA+ model `Wakeup` takes them: Park starts a wait with a new
// sequence, and a claim succeeds only on the wait whose sequence it names.
pub open spec fn wake_transition(
    state: State,
    sequence: u64,
    event: Event,
) -> Option<(State, u64)> {
    match event {
        Event::Park => if state == State::Running && sequence < crate::MAX_SEQUENCE {
            Some((State::Parking, (sequence + 1) as u64))
        } else {
            None
        },
        Event::Block => if state == State::Parking {
            Some((State::Blocked, sequence))
        } else {
            None
        },
        Event::Resume => if state == State::Woken {
            Some((State::Running, sequence))
        } else {
            None
        },
        Event::Place => if state == State::Waking {
            Some((State::Running, sequence))
        } else {
            None
        },
        Event::Claim { sequence: claimed } => if claimed != sequence {
            None
        } else if state == State::Parking {
            Some((State::Woken, sequence))
        } else if state == State::Blocked {
            Some((State::Waking, sequence))
        } else {
            None
        },
        Event::Exit => if state == State::Running {
            Some((State::Dead, sequence))
        } else {
            None
        },
    }
}

// A wait is past its claim once the waker owns the wakeup: no move of that wait blocks again.
pub open spec fn past_claim(state: State) -> bool {
    state == State::Woken || state == State::Waking || state == State::Running || state
        == State::Dead
}

// Wakeup: if a waker claims a thread's wait after the thread registered, the thread is not left
// blocked. A claim of the registered wait succeeds, and from then on no move of that wait reaches
// Parking or Blocked.
pub open spec fn claim_after_registration_never_left_blocked(
    state: State,
    sequence: u64,
    event: Event,
) -> bool {
    &&& (state == State::Parking || state == State::Blocked) ==> wake_transition(
        state,
        sequence,
        Event::Claim { sequence },
    ) is Some
    &&& match wake_transition(state, sequence, event) {
        Some((next, later)) => {
            &&& event is Claim ==> next == State::Woken || next == State::Waking
            &&& past_claim(state) && later == sequence ==> past_claim(next)
        },
        None => true,
    }
}

// Wakeup: a claim that names an old wait never completes a later one. The sequence only grows, and
// a claim takes effect only on the wait whose sequence it names.
pub open spec fn stale_claim_never_completes_a_later_wait(
    state: State,
    sequence: u64,
    event: Event,
) -> bool {
    match wake_transition(state, sequence, event) {
        Some((_, later)) => {
            &&& sequence <= later
            &&& event is Claim ==> event == Event::Claim { sequence }
        },
        None => true,
    }
}

// Wakeup: a wait completes once. Only a claimed wait reaches Running, a claim succeeds only before
// one has, and once the wait is complete no move of that wait reaches Running again.
pub open spec fn wait_completes_once(state: State, sequence: u64, event: Event) -> bool {
    match wake_transition(state, sequence, event) {
        Some((next, later)) => {
            &&& next == State::Running ==> state == State::Woken || state == State::Waking
            &&& event is Claim ==> state == State::Parking || state == State::Blocked
            &&& (state == State::Running || state == State::Dead) && later == sequence ==> next
                == State::Dead
        },
        None => true,
    }
}

pub proof fn lemma_wakeup(state: State, sequence: u64, event: Event)
    ensures
        claim_after_registration_never_left_blocked(state, sequence, event),
        stale_claim_never_completes_a_later_wait(state, sequence, event),
        wait_completes_once(state, sequence, event),
{
}

pub(crate) proof fn lemma_with_keeps_the_sequence(bits: u64, code: u64)
    requires
        code <= 5,
    ensures
        ((bits & !7u64) | code) & 7u64 == code,
        ((bits & !7u64) | code) >> 3u32 == bits >> 3u32,
{
    assert(((bits & !7u64) | code) & 7u64 == code && ((bits & !7u64) | code) >> 3u32 == bits
        >> 3u32) by (bit_vector)
        requires
            code <= 5,
    ;
}

pub(crate) proof fn lemma_park_starts_a_sequence(sequence: u64, code: u64)
    requires
        sequence <= crate::MAX_SEQUENCE,
        code <= 5,
    ensures
        ((sequence << 3u32) | code) & 7u64 == code,
        ((sequence << 3u32) | code) >> 3u32 == sequence,
{
    assert(u64::MAX >> 3u32 == 0x1fff_ffff_ffff_ffffu64) by (bit_vector);
    assert(((sequence << 3u32) | code) & 7u64 == code && ((sequence << 3u32) | code) >> 3u32
        == sequence) by (bit_vector)
        requires
            sequence <= 0x1fff_ffff_ffff_ffffu64,
            code <= 5,
    ;
}

pub(crate) proof fn lemma_idle_and_allowed_is_allowed(idle: u64, allowed: u64, cpu: u32)
    requires
        cpu < 64,
        (idle & allowed) & (1u64 << cpu) != 0,
    ensures
        allowed & (1u64 << cpu) != 0,
{
    assert((idle & allowed) & (1u64 << cpu) != 0 ==> allowed & (1u64 << cpu) != 0) by (bit_vector);
}

} // verus!
