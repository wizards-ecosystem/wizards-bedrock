use crate::event::{KeyEvent, KeyState};
use crate::key::Key;
use crate::{set1, table};

// Swallowed wherever they arrive: a command can be answered while a key sequence is in flight.
mod protocol {
    pub(super) const BREAK: u8 = 0xF0;
    pub(super) const EXTENDED: u8 = 0xE0;
    pub(super) const PAUSE: u8 = 0xE1;
    pub(super) const ACK: u8 = 0xFA;
    // In set 1 also left shift's release, so the reset path reads it before scanning starts.
    pub(super) const SELF_TEST_PASSED: u8 = 0xAA;
    pub(super) const ECHO: u8 = 0xEE;
    pub(super) const RESEND: u8 = 0xFE;
    pub(super) const OVERRUN: u8 = 0x00;
    pub(super) const ERROR: u8 = 0xFF;

    pub(super) const fn is_reply(byte: u8) -> bool {
        matches!(
            byte,
            ACK | SELF_TEST_PASSED | ECHO | RESEND | OVERRUN | ERROR
        )
    }
}

const FAKE_SHIFTS: [u8; 2] = [0x12, 0x59];

// Pause sends this whole sequence on press and nothing on release.
const PAUSE_SEQUENCE: [u8; 8] = [0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];

const SET_1_BREAK: u8 = 0x80;

const SET_1_PAUSE_SEQUENCE: [u8; 6] = [0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5];

const SET_1_FAKE_SHIFTS: [u8; 4] = [0x2A, 0xAA, 0x36, 0xB6];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Set {
    Two,
    One,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Idle,
    Break,
    Extended,
    ExtendedBreak,
    Pause(u8),
}

#[derive(Debug)]
pub struct Decoder {
    state: State,
    set: Set,
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            state: State::Idle,
            set: Set::Two,
        }
    }

    pub const fn translated() -> Self {
        Self {
            state: State::Idle,
            set: Set::One,
        }
    }

    pub const fn reads_translated(&self) -> bool {
        matches!(self.set, Set::One)
    }

    pub fn feed(&mut self, byte: u8) -> Option<KeyEvent> {
        let reply = match self.set {
            Set::Two => protocol::is_reply(byte),
            Set::One => protocol::is_reply(byte) && byte != protocol::SELF_TEST_PASSED,
        };
        if reply {
            return None;
        }
        let (state, event) = match self.set {
            Set::Two => self.transition(byte),
            Set::One => self.transition_set_1(byte),
        };
        self.state = state;
        event
    }

    fn transition_set_1(&self, byte: u8) -> (State, Option<KeyEvent>) {
        let code = byte & !SET_1_BREAK;
        let broke = byte & SET_1_BREAK != 0;
        let settle = |key: Option<Key>| if broke { released(key) } else { pressed(key) };

        match self.state {
            State::Idle => match byte {
                protocol::EXTENDED => (State::Extended, None),
                protocol::PAUSE => (State::Pause(1), None),
                _ => (State::Idle, settle(set1::key_for(code))),
            },
            State::Extended => {
                if SET_1_FAKE_SHIFTS.contains(&byte) {
                    (State::Idle, None)
                } else {
                    (State::Idle, settle(set1::extended_key_for(code)))
                }
            }
            State::Pause(matched) => pause_step(&SET_1_PAUSE_SEQUENCE, matched, byte),
            State::Break | State::ExtendedBreak => (State::Idle, None),
        }
    }

    fn transition(&self, byte: u8) -> (State, Option<KeyEvent>) {
        match self.state {
            State::Idle => match byte {
                protocol::BREAK => (State::Break, None),
                protocol::EXTENDED => (State::Extended, None),
                protocol::PAUSE => (State::Pause(1), None),
                code => (State::Idle, pressed(table::key_for(code))),
            },
            State::Break => (State::Idle, released(table::key_for(byte))),
            State::Extended => match byte {
                protocol::BREAK => (State::ExtendedBreak, None),
                code if FAKE_SHIFTS.contains(&code) => (State::Idle, None),
                code => (State::Idle, pressed(table::extended_key_for(code))),
            },
            State::ExtendedBreak => match byte {
                code if FAKE_SHIFTS.contains(&code) => (State::Idle, None),
                code => (State::Idle, released(table::extended_key_for(code))),
            },
            State::Pause(matched) => pause_step(&PAUSE_SEQUENCE, matched, byte),
        }
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

fn pause_step(sequence: &[u8], matched: u8, byte: u8) -> (State, Option<KeyEvent>) {
    if sequence.get(usize::from(matched)) != Some(&byte) {
        return (State::Idle, None);
    }
    let matched = matched + 1;
    if usize::from(matched) == sequence.len() {
        (State::Idle, pressed(Some(Key::Pause)))
    } else {
        (State::Pause(matched), None)
    }
}

fn pressed(key: Option<Key>) -> Option<KeyEvent> {
    key.map(|key| KeyEvent {
        key,
        state: KeyState::Pressed,
    })
}

fn released(key: Option<Key>) -> Option<KeyEvent> {
    key.map(|key| KeyEvent {
        key,
        state: KeyState::Released,
    })
}

#[cfg(test)]
mod tests;
