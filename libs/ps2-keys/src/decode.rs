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
use crate::table;

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

/// Pause sends this whole sequence on press and nothing on release — the
/// one key with no break code.
const PAUSE_SEQUENCE: [u8; 8] = [0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];

/// Where the machine stands between bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The scancode set 2 decoder.
#[derive(Debug)]
pub struct Decoder {
    state: State,
}

impl Decoder {
    /// A decoder with nothing in flight.
    pub const fn new() -> Self {
        Self { state: State::Idle }
    }

    /// Feed one byte; an event comes out when one is complete.
    ///
    /// Total: every byte in every state has a defined transition, and a byte
    /// that fits no sequence resets the machine to idle so the next key
    /// still decodes. Protocol replies are transparent wherever they arrive:
    /// they produce no event and leave an in-flight key sequence intact.
    pub fn feed(&mut self, byte: u8) -> Option<KeyEvent> {
        if protocol::is_reply(byte) {
            return None;
        }
        let (state, event) = self.transition(byte);
        self.state = state;
        event
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
mod tests {
    use super::*;

    /// Drive one sequence byte by byte, asserting silence at every step but
    /// the last — which is the shape of the real driver's loop.
    fn decode(decoder: &mut Decoder, sequence: &[u8]) -> Option<KeyEvent> {
        let (last, rest) = sequence.split_last().expect("empty sequence");
        for &byte in rest {
            assert_eq!(decoder.feed(byte), None, "early event at {byte:#04x}");
        }
        decoder.feed(*last)
    }

    fn event(key: Key, state: KeyState) -> Option<KeyEvent> {
        Some(KeyEvent { key, state })
    }

    #[test]
    fn a_letter_make_is_a_press() {
        let mut decoder = Decoder::new();
        assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
    }

    #[test]
    fn a_break_prefix_makes_the_next_code_a_release() {
        let mut decoder = Decoder::new();
        assert_eq!(
            decode(&mut decoder, &[0xF0, 0x1C]),
            event(Key::A, KeyState::Released)
        );
    }

    #[test]
    fn an_extended_make_decodes() {
        let mut decoder = Decoder::new();
        assert_eq!(
            decode(&mut decoder, &[0xE0, 0x74]),
            event(Key::ArrowRight, KeyState::Pressed)
        );
    }

    #[test]
    fn an_extended_break_decodes() {
        let mut decoder = Decoder::new();
        assert_eq!(
            decode(&mut decoder, &[0xE0, 0xF0, 0x74]),
            event(Key::ArrowRight, KeyState::Released)
        );
    }

    #[test]
    fn the_pause_sequence_yields_one_press_and_no_break() {
        let mut decoder = Decoder::new();
        assert_eq!(
            decode(&mut decoder, &PAUSE_SEQUENCE),
            event(Key::Pause, KeyState::Pressed)
        );
        assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
    }

    /// The wire form of a bare PrintScreen press: fake shift, then the key.
    #[test]
    fn print_screen_make_is_one_event() {
        let mut decoder = Decoder::new();
        let mut events = [0xE0, 0x12, 0xE0, 0x7C]
            .into_iter()
            .filter_map(|byte| decoder.feed(byte));

        assert_eq!(events.next(), event(Key::PrintScreen, KeyState::Pressed));
        assert_eq!(events.next(), None);
    }

    /// And its release: the key, then the fake shift's break.
    #[test]
    fn print_screen_break_is_one_event() {
        let mut decoder = Decoder::new();
        let mut events = [0xE0, 0xF0, 0x7C, 0xE0, 0xF0, 0x12]
            .into_iter()
            .filter_map(|byte| decoder.feed(byte));

        assert_eq!(events.next(), event(Key::PrintScreen, KeyState::Released));
        assert_eq!(events.next(), None);
    }

    #[test]
    fn protocol_bytes_do_not_disturb_idle() {
        let mut decoder = Decoder::new();
        for byte in [0xFA, 0xAA, 0xEE, 0xFE, 0x00, 0xFF] {
            assert_eq!(decoder.feed(byte), None, "{byte:#04x}");
        }
        assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
    }

    #[test]
    fn protocol_replies_are_transparent_inside_every_sequence() {
        let replies = [0xFA, 0xAA, 0xEE, 0xFE, 0x00, 0xFF];

        for reply in replies {
            let mut decoder = Decoder::new();
            assert_eq!(decoder.feed(0xF0), None);
            assert_eq!(decoder.feed(reply), None);
            assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Released));

            let mut decoder = Decoder::new();
            assert_eq!(decoder.feed(0xE0), None);
            assert_eq!(decoder.feed(reply), None);
            assert_eq!(
                decoder.feed(0x74),
                event(Key::ArrowRight, KeyState::Pressed)
            );

            let mut decoder = Decoder::new();
            assert_eq!(decode(&mut decoder, &[0xE0, 0xF0]), None);
            assert_eq!(decoder.feed(reply), None);
            assert_eq!(
                decoder.feed(0x74),
                event(Key::ArrowRight, KeyState::Released)
            );

            for split in 1..PAUSE_SEQUENCE.len() {
                let mut decoder = Decoder::new();
                for byte in &PAUSE_SEQUENCE[..split] {
                    assert_eq!(decoder.feed(*byte), None);
                }
                assert_eq!(decoder.feed(reply), None);
                assert_eq!(
                    decode(&mut decoder, &PAUSE_SEQUENCE[split..]),
                    event(Key::Pause, KeyState::Pressed),
                    "reply {reply:#04x} after {split} Pause bytes"
                );
            }
        }
    }

    #[test]
    fn an_unknown_make_code_is_swallowed() {
        let mut decoder = Decoder::new();
        assert_eq!(decoder.feed(0xF7), None);
        assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
    }

    #[test]
    fn an_unknown_break_recovers() {
        let mut decoder = Decoder::new();
        assert_eq!(decode(&mut decoder, &[0xF0, 0xF7]), None);
        assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
    }

    #[test]
    fn a_corrupt_pause_sequence_recovers() {
        let mut decoder = Decoder::new();
        assert_eq!(decode(&mut decoder, &[0xE1, 0x14, 0x99]), None);
        assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
    }

    #[test]
    fn f7_sits_above_the_seven_bit_boundary() {
        let mut decoder = Decoder::new();
        assert_eq!(decoder.feed(0x83), event(Key::F7, KeyState::Pressed));
        assert_eq!(
            decode(&mut decoder, &[0xF0, 0x83]),
            event(Key::F7, KeyState::Released)
        );
    }

    /// A held key repeats as bare makes with no breaks between; each one is
    /// a fresh press, which is what a future input stack debounces, not this
    /// crate.
    #[test]
    fn typematic_repeat_is_a_press_per_make() {
        let mut decoder = Decoder::new();
        for _ in 0..3 {
            assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
        }
        assert_eq!(
            decode(&mut decoder, &[0xF0, 0x1C]),
            event(Key::A, KeyState::Released)
        );
    }

    /// Totality: every byte, fed in every reachable prefix state, neither
    /// panics nor wedges the machine — a letter still decodes afterwards,
    /// possibly on the second try when the byte itself opened a new prefix.
    ///
    /// The Pause prefixes are derived from the sequence constant, so every
    /// intermediate `Pause(n)` state — home of the crate's one indexed
    /// access — is visited even if the sequence changes. The list once
    /// skipped three of them by hand-enumeration, which is the mistake this
    /// construction removes.
    #[test]
    fn no_byte_in_no_state_panics_or_wedges() {
        let other_prefixes: [&[u8]; 4] = [&[], &[0xF0], &[0xE0], &[0xE0, 0xF0]];
        let pause_prefixes = (1..PAUSE_SEQUENCE.len()).map(|matched| &PAUSE_SEQUENCE[..matched]);

        for prefix in other_prefixes.into_iter().chain(pause_prefixes) {
            for byte in 0..=u8::MAX {
                let mut decoder = Decoder::new();
                for &prefix_byte in prefix {
                    let _ = decoder.feed(prefix_byte);
                }
                let _ = decoder.feed(byte);

                let recovered = (0..3).any(|_| {
                    decoder.feed(0x1C)
                        == Some(KeyEvent {
                            key: Key::A,
                            state: KeyState::Pressed,
                        })
                });
                assert!(recovered, "wedged by {byte:#04x} after {prefix:02x?}");
            }
        }
    }
}
