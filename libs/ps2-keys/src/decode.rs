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

// Both bytes are also absent from `table::extended_key_for`, so today the
// guard and the table agree and no test can tell which suppressed a fake
// shift. The guard is what keeps the behaviour if either byte is ever added
// there — they are real keys in the *base* table, which is exactly why a
// future edit might (D-124).

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Set {
    /// What the keyboard sends when nothing rewrites it.
    Two,
    /// What a translating controller delivers.
    One,
}

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
        // **`0xAA` is not a reply in set 1.** It is `0x2A | 0x80`, the release
        // of the left shift, and swallowing it would leave shift held down for
        // ever after the first capital letter. The self-test result it collides
        // with is read by the reset path directly, before scanning is enabled,
        // so it never reaches a decoder (D-179).
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
            // Set 1 has no break prefix, so these two cannot be reached from
            // `transition_set_1` — a decoder never enters them. Resetting is
            // the same answer the machine gives any byte that fits no
            // sequence.
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

    /// Set 1's release is the high bit, and it is not a prefix.
    #[test]
    fn set_1_puts_the_release_in_the_high_bit() {
        let mut decoder = Decoder::translated();
        assert_eq!(decoder.feed(0x1E), event(Key::A, KeyState::Pressed));
        assert_eq!(decoder.feed(0x9E), event(Key::A, KeyState::Released));
    }

    /// The extended table, both directions, through the same high bit.
    #[test]
    fn set_1_extended_codes_decode_either_way() {
        let mut decoder = Decoder::translated();
        assert_eq!(
            decode(&mut decoder, &[0xE0, 0x4B]),
            event(Key::ArrowLeft, KeyState::Pressed)
        );
        assert_eq!(
            decode(&mut decoder, &[0xE0, 0xCB]),
            event(Key::ArrowLeft, KeyState::Released)
        );
    }

    /// PrintScreen arrives wrapped in shifts nothing pressed, as in set 2.
    #[test]
    fn set_1_discards_the_fake_shifts_around_print_screen() {
        let mut decoder = Decoder::translated();
        assert_eq!(
            decode(&mut decoder, &[0xE0, 0x2A, 0xE0, 0x37]),
            event(Key::PrintScreen, KeyState::Pressed)
        );
    }

    /// Six bytes rather than eight, and still no release.
    #[test]
    fn set_1_pause_is_six_bytes() {
        let mut decoder = Decoder::translated();
        assert_eq!(
            decode(&mut decoder, &SET_1_PAUSE_SEQUENCE),
            event(Key::Pause, KeyState::Pressed)
        );
    }

    /// **The collision, decided.**
    ///
    /// `0xAA` is the self-test result *and* `0x2A | 0x80`, the left shift's
    /// release. Swallowing it as a reply would leave shift held down for ever
    /// after the first capital letter — so in set 1 it is a key, and the
    /// self-test result is read by the reset path before scanning is enabled
    /// where no decoder can see it (D-179).
    #[test]
    fn set_1_reads_aa_as_a_shift_release_not_a_self_test_result() {
        let mut decoder = Decoder::translated();
        assert_eq!(decoder.feed(0x2A), event(Key::ShiftLeft, KeyState::Pressed));
        assert_eq!(
            decoder.feed(0xAA),
            event(Key::ShiftLeft, KeyState::Released)
        );

        // And set 2, where `0xAA` is unambiguous, still swallows it.
        let mut set_two = Decoder::new();
        assert_eq!(set_two.feed(protocol::SELF_TEST_PASSED), None);
    }

    /// The replies that are not ambiguous stay transparent in set 1 too.
    #[test]
    fn set_1_still_swallows_the_unambiguous_replies() {
        let mut decoder = Decoder::translated();
        for reply in [protocol::ACK, protocol::ECHO, protocol::RESEND] {
            assert_eq!(decoder.feed(reply), None, "{reply:#04x} is not a key");
        }
        // And a key still decodes after them.
        assert_eq!(decoder.feed(0x1E), event(Key::A, KeyState::Pressed));
    }

    /// A shift held across a letter types a capital, through the set 1 path.
    ///
    /// The end-to-end shape the driver runs: decoder to shift state to layout.
    /// It is the sequence that would have typed nothing on VMware (D-179).
    #[test]
    fn set_1_types_a_capital_through_the_whole_path() {
        let mut decoder = Decoder::translated();
        let mut shift = crate::shift::Shift::new();
        let mut typed = None;
        for byte in [0x2A, 0x1E, 0x9E, 0xAA] {
            if let Some(key) = decoder.feed(byte) {
                shift.observe(key);
                if key.state == KeyState::Pressed {
                    typed = typed.or(crate::us::char_for(key.key, shift));
                }
            }
        }
        assert_eq!(typed, Some('A'));
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
        // Transcribed, not `PAUSE_SEQUENCE`. Feeding the constant to a decoder
        // that compares against the constant is `x[i] == x[i]`, true for any
        // array of any length — the constant truncated to four bytes passed
        // all 31 tests, and the decoder then read the trailing `F0 14 F0 77`
        // as two phantom release events (D-105).
        const WIRE: [u8; 8] = [0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];
        assert_eq!(PAUSE_SEQUENCE, WIRE, "the sequence is not the wire form");

        let mut decoder = Decoder::new();
        assert_eq!(
            decode(&mut decoder, &WIRE),
            event(Key::Pause, KeyState::Pressed)
        );
        assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
    }

    /// A short prefix of the sequence yields nothing, and the byte after a
    /// complete one is an ordinary key.
    ///
    /// The length is the half nothing pinned: a decoder that stopped early
    /// would emit `Pause` and then decode the remaining bytes as keys.
    #[test]
    fn no_proper_prefix_of_the_pause_sequence_decodes() {
        const WIRE: [u8; 8] = [0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];

        for split in 1..WIRE.len() {
            let mut decoder = Decoder::new();
            for byte in &WIRE[..split] {
                assert_eq!(decoder.feed(*byte), None, "byte {byte:#04x} of {split}");
            }
        }

        // And a wrong byte anywhere in the middle abandons the sequence
        // rather than completing it.
        for corrupt in 1..WIRE.len() {
            let mut decoder = Decoder::new();
            let mut wrong = WIRE;
            wrong[corrupt] = 0x1C;
            let events: Vec<_> = wrong.iter().map(|byte| decoder.feed(*byte)).collect();
            assert!(
                !events.contains(&event(Key::Pause, KeyState::Pressed)),
                "byte {corrupt} corrupted and Pause still fired"
            );
        }
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

    /// **Both** fake shifts are swallowed, in both directions.
    ///
    /// The tests reached `FAKE_SHIFTS` only with `0x12`, so its second element
    /// was never exercised. Widening this to both did **not** make the
    /// constant load-bearing, and saying so is the point: `extended_key_for`
    /// has no arm for either byte, so the guard and the table give the same
    /// answer and neither can be shown to be the one giving it (D-124).
    ///
    /// The test is kept because it pins the *behaviour* — `E0 12` and `E0 59`
    /// produce nothing, whichever layer decides — and because the guard is
    /// what survives someone adding those bytes to the extended table, which
    /// is a plausible edit: they are real keys in the base one.
    #[test]
    fn both_fake_shifts_are_swallowed() {
        for fake in [0x12u8, 0x59] {
            let mut decoder = Decoder::new();
            // The make: `E0 <fake>` yields nothing at all.
            assert_eq!(decoder.feed(0xE0), None, "{fake:#04x} make prefix");
            assert_eq!(decoder.feed(fake), None, "{fake:#04x} make");

            // The break: `E0 F0 <fake>` likewise.
            assert_eq!(decoder.feed(0xE0), None, "{fake:#04x} break prefix");
            assert_eq!(decoder.feed(0xF0), None, "{fake:#04x} break marker");
            assert_eq!(decoder.feed(fake), None, "{fake:#04x} break");

            // And the decoder is back at rest, not stuck in a state.
            assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
        }
    }
}
