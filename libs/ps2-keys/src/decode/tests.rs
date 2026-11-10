use super::*;

/// Drive one sequence byte by byte as the driver's loop does, asserting silence until the last.
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
fn set_1_puts_the_release_in_the_high_bit() {
    assert!(!Decoder::new().reads_translated());
    let mut decoder = Decoder::translated();
    assert!(decoder.reads_translated());
    assert_eq!(decoder.feed(0x1E), event(Key::A, KeyState::Pressed));
    assert_eq!(decoder.feed(0x9E), event(Key::A, KeyState::Released));
}

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

#[test]
fn set_1_discards_the_fake_shifts_around_print_screen() {
    let mut decoder = Decoder::translated();
    assert_eq!(
        decode(&mut decoder, &[0xE0, 0x2A, 0xE0, 0x37]),
        event(Key::PrintScreen, KeyState::Pressed)
    );
}

#[test]
fn set_1_pause_is_six_bytes() {
    let mut decoder = Decoder::translated();
    assert_eq!(
        decode(&mut decoder, &SET_1_PAUSE_SEQUENCE),
        event(Key::Pause, KeyState::Pressed)
    );
}

/// Swallowing `AA` in set 1 would hold shift down for ever.
/// The reset path reads the self-test result before scanning, where no decoder sees it.
#[test]
fn set_1_reads_aa_as_a_shift_release_not_a_self_test_result() {
    let mut decoder = Decoder::translated();
    assert_eq!(decoder.feed(0x2A), event(Key::ShiftLeft, KeyState::Pressed));
    assert_eq!(
        decoder.feed(0xAA),
        event(Key::ShiftLeft, KeyState::Released)
    );

    let mut set_two = Decoder::new();
    assert_eq!(set_two.feed(protocol::SELF_TEST_PASSED), None);
}

#[test]
fn set_1_still_swallows_the_unambiguous_replies() {
    let mut decoder = Decoder::translated();
    for reply in [protocol::ACK, protocol::ECHO, protocol::RESEND] {
        assert_eq!(decoder.feed(reply), None, "{reply:#04x} is not a key");
    }
    assert_eq!(decoder.feed(0x1E), event(Key::A, KeyState::Pressed));
}

/// Decoder to shift state to layout, the sequence that would have typed nothing on VMware.
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

/// Transcribed, since a truncated `PAUSE_SEQUENCE` compared with itself passed every test
/// and left phantom releases.
#[test]
fn the_pause_sequence_yields_one_press_and_no_break() {
    const WIRE: [u8; 8] = [0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];
    assert_eq!(PAUSE_SEQUENCE, WIRE, "the sequence is not the wire form");

    let mut decoder = Decoder::new();
    assert_eq!(
        decode(&mut decoder, &WIRE),
        event(Key::Pause, KeyState::Pressed)
    );
    assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
}

/// A decoder that stopped early would emit `Pause` and then decode the remaining bytes as keys.
#[test]
fn no_proper_prefix_of_the_pause_sequence_decodes() {
    const WIRE: [u8; 8] = [0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];

    for split in 1..WIRE.len() {
        let mut decoder = Decoder::new();
        for byte in &WIRE[..split] {
            assert_eq!(decoder.feed(*byte), None, "byte {byte:#04x} of {split}");
        }
    }

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

#[test]
fn print_screen_make_is_one_event() {
    let mut decoder = Decoder::new();
    let mut events = [0xE0, 0x12, 0xE0, 0x7C]
        .into_iter()
        .filter_map(|byte| decoder.feed(byte));

    assert_eq!(
        core::iter::Iterator::next(&mut events),
        event(Key::PrintScreen, KeyState::Pressed)
    );
    assert_eq!(core::iter::Iterator::next(&mut events), None);
}

#[test]
fn print_screen_break_is_one_event() {
    let mut decoder = Decoder::new();
    let mut events = [0xE0, 0xF0, 0x7C, 0xE0, 0xF0, 0x12]
        .into_iter()
        .filter_map(|byte| decoder.feed(byte));

    assert_eq!(
        core::iter::Iterator::next(&mut events),
        event(Key::PrintScreen, KeyState::Released)
    );
    assert_eq!(core::iter::Iterator::next(&mut events), None);
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

/// A held key repeats as bare makes, each a fresh press for a future input stack to debounce.
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

/// Recovery may take a retry when the byte itself opened a prefix.
/// Pause prefixes come from the constant, so every indexed `Pause(n)` state stays covered.
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

/// Pins wire behaviour only, since the extended table also omits both fake shifts.
#[test]
fn both_fake_shifts_are_swallowed() {
    for fake in [0x12u8, 0x59] {
        let mut decoder = Decoder::new();
        assert_eq!(decoder.feed(0xE0), None, "{fake:#04x} make prefix");
        assert_eq!(decoder.feed(fake), None, "{fake:#04x} make");

        assert_eq!(decoder.feed(0xE0), None, "{fake:#04x} break prefix");
        assert_eq!(decoder.feed(0xF0), None, "{fake:#04x} break marker");
        assert_eq!(decoder.feed(fake), None, "{fake:#04x} break");

        assert_eq!(decoder.feed(0x1C), event(Key::A, KeyState::Pressed));
    }
}
