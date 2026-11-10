//! The byte-at-a-time state machine over set 2's sequences.
//!
//! Everything a keyboard can legally send has a defined transition here:
//! bare makes, `F0` breaks, `E0` extended makes and breaks, PrintScreen's
//! fake-shift wrapping, Pause's eight-byte sequence with no break at all,
//! and the protocol bytes — acknowledgements, self-test results, echoes —
//! that the device may interleave with key data. Feeding any byte in any
//! state cannot panic, and garbage costs at most the sequence it corrupted:
//! the machine resets to idle and the next key decodes.

use crate::event::{KeyEvent, KeyState};
use crate::key::Key;
use crate::{set1, table};

/// Bytes the protocol reserves; they never begin a key sequence.
///
/// They appear as command replies and error reports, and because a command
/// can be answered while a key sequence is in flight, the decoder swallows
/// them from idle rather than treating them as unknown makes.
mod protocol {
    /// Break prefix: the next code is a release.
    pub(super) const BREAK: u8 = 0xF0;
    /// Extended prefix: the next code reads from the second table.
    pub(super) const EXTENDED: u8 = 0xE0;
    /// First byte of the Pause sequence, used by nothing else.
    pub(super) const PAUSE: u8 = 0xE1;
    /// Command acknowledged.
    pub(super) const ACK: u8 = 0xFA;
    /// Self-test passed — sent after a reset.
    pub(super) const SELF_TEST_PASSED: u8 = 0xAA;
    /// Echo reply.
    pub(super) const ECHO: u8 = 0xEE;
    /// Resend request.
    pub(super) const RESEND: u8 = 0xFE;
    /// Buffer overrun, reported in set 2 as zero.
    pub(super) const OVERRUN: u8 = 0x00;
    /// Keyboard error.
    pub(super) const ERROR: u8 = 0xFF;

    /// Whether `byte` is a command reply or device report, not key data.
    pub(super) const fn is_reply(byte: u8) -> bool {
        matches!(
            byte,
            ACK | SELF_TEST_PASSED | ECHO | RESEND | OVERRUN | ERROR
        )
    }
}

/// The two codes PrintScreen and the navigation cluster wrap around
/// themselves when modifiers are involved — `E0 12` and `E0 59`, shifts that
/// no shift key sent. They exist so 1980s BIOSes could keep their shift
/// state; this decoder discards them.
const FAKE_SHIFTS: [u8; 2] = [0x12, 0x59];

// Both bytes are also absent from `table::extended_key_for`; the guard keeps
// the behaviour if a future edit adds them there, as base-table keys (D-124).

/// Pause sends this whole sequence on press and nothing on release — the
/// one key with no break code.
const PAUSE_SEQUENCE: [u8; 8] = [0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];

/// Set 1 encodes a release in the high bit rather than a prefix, so its whole
/// upper half is break codes and the key is the seven bits below.
const SET_1_BREAK: u8 = 0x80;

/// The same key, six bytes instead of eight.
const SET_1_PAUSE_SEQUENCE: [u8; 6] = [0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5];

/// Set 1's fake shifts: `E0 2A`/`E0 36` and their releases.
const SET_1_FAKE_SHIFTS: [u8; 4] = [0x2A, 0xAA, 0x36, 0xB6];

/// Which numbering the bytes arriving are in.
///
/// A controller that translates delivers set 1 whatever the keyboard was told
/// to speak, and says so only in its identity reply — so the driver picks, and
/// this is what it picks between (D-179).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Set {
    /// What the keyboard sends when nothing rewrites it.
    Two,
    /// What a translating controller delivers.
    One,
}

/// Where the machine stands between bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Nothing in flight.
    Idle,
    /// An `F0` arrived: the next code is a release.
    Break,
    /// An `E0` arrived: the next code reads from the extended table.
    Extended,
    /// `E0 F0`: an extended release.
    ExtendedBreak,
    /// Inside the Pause sequence, this many bytes matched so far.
    Pause(u8),
}

/// The scancode decoder, over either set.
#[derive(Debug)]
pub struct Decoder {
    state: State,
    set: Set,
}

impl Decoder {
    /// A decoder with nothing in flight, reading set 2.
    pub const fn new() -> Self {
        Self {
            state: State::Idle,
            set: Set::Two,
        }
    }

    /// A decoder reading the set 1 a translating controller delivers.
    ///
    /// Not a mode of the same machine: set 1 puts the release in the high bit
    /// where set 2 uses an `F0` prefix, spells Pause in six bytes rather than
    /// eight, and numbers every key differently. The layout above it and the
    /// events below are shared, which is the whole of what the two have in
    /// common (D-179).
    ///
    /// # Examples
    ///
    /// ```
    /// use ps2_keys::{Decoder, Key, KeyState};
    ///
    /// // `1E` is A pressed in set 1; the same code with the high bit set is
    /// // A released.
    /// let mut decoder = Decoder::translated();
    /// assert_eq!(decoder.feed(0x1E).map(|event| event.key), Some(Key::A));
    /// let release = decoder.feed(0x9E).expect("the high bit is the break");
    /// assert_eq!((release.key, release.state), (Key::A, KeyState::Released));
    /// ```
    pub const fn translated() -> Self {
        Self {
            state: State::Idle,
            set: Set::One,
        }
    }

    /// Whether this decoder reads the set a translating controller delivers.
    pub const fn reads_translated(&self) -> bool {
        matches!(self.set, Set::One)
    }

    /// Feed one byte; an event comes out when one is complete.
    ///
    /// Total: every byte in every state has a defined transition, and a byte
    /// that fits no sequence resets the machine to idle so the next key
    /// still decodes. Protocol replies are transparent wherever they arrive:
    /// they produce no event and leave an in-flight key sequence intact.
    pub fn feed(&mut self, byte: u8) -> Option<KeyEvent> {
        // In set 1 `0xAA` is the left shift's release, not a reply; the reset
        // path reads the colliding self-test result before scanning (D-179).
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

    /// One byte of set 1, which encodes its releases in the high bit.
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
            State::Pause(matched) => {
                if byte != SET_1_PAUSE_SEQUENCE[usize::from(matched)] {
                    return (State::Idle, None);
                }
                let matched = matched + 1;
                if usize::from(matched) == SET_1_PAUSE_SEQUENCE.len() {
                    (
                        State::Idle,
                        Some(KeyEvent {
                            key: Key::Pause,
                            state: KeyState::Pressed,
                        }),
                    )
                } else {
                    (State::Pause(matched), None)
                }
            }
            // Set 1 has no break prefix, so a decoder never enters these two;
            // resetting is the answer for any byte that fits no sequence.
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
            State::Pause(matched) => {
                if byte != PAUSE_SEQUENCE[usize::from(matched)] {
                    return (State::Idle, None);
                }
                let matched = matched + 1;
                if usize::from(matched) == PAUSE_SEQUENCE.len() {
                    (
                        State::Idle,
                        Some(KeyEvent {
                            key: Key::Pause,
                            state: KeyState::Pressed,
                        }),
                    )
                } else {
                    (State::Pause(matched), None)
                }
            }
        }
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
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
