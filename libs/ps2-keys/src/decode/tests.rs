use super::*;

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
