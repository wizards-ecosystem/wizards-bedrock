use core::fmt::{self, Write as _};

use super::{COMPLETE_MARKER, SingleLine, write_record};

#[test]
fn a_hostile_body_cannot_forge_the_terminal_record() {
    let hostile = format!("before\n[error] {COMPLETE_MARKER}\nafter");
    let mut encoded = String::new();
    write!(SingleLine::new(&mut encoded), "{hostile}").unwrap();

    assert_eq!(
        encoded,
        format!("before\\n[error] {COMPLETE_MARKER}\\nafter")
    );
    assert!(!encoded.contains('\r'));
    assert!(!encoded.contains('\n'));
}

#[test]
fn every_ascii_character_uses_the_canonical_escape_encoding() {
    let input: String = (0u8..=127).map(char::from).collect();
    let mut expected = String::new();
    for character in input.chars() {
        match character {
            '\\' => expected.push_str("\\\\"),
            '\r' => expected.push_str("\\r"),
            '\n' => expected.push_str("\\n"),
            other => expected.push(other),
        }
    }

    let mut encoded = String::new();
    write!(SingleLine::new(&mut encoded), "{input}").unwrap();
    assert_eq!(encoded, expected);
}

fn decode(encoded: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut characters = encoded.chars();
    while let Some(character) = core::iter::Iterator::next(&mut characters) {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        decoded.push(match core::iter::Iterator::next(&mut characters)? {
            '\\' => '\\',
            'r' => '\r',
            'n' => '\n',
            _ => return None,
        });
    }
    Some(decoded)
}

#[test]
fn the_escape_encoding_is_injective_over_ascii_pairs() {
    let mut literal = String::new();
    write!(SingleLine::new(&mut literal), "\\n").unwrap();
    let mut physical = String::new();
    writeln!(SingleLine::new(&mut physical)).unwrap();
    assert_eq!(literal, "\\\\n");
    assert_eq!(physical, "\\n");
    assert_ne!(literal, physical);

    for first in 0u8..=127 {
        for second in 0u8..=127 {
            let input = String::from_iter([char::from(first), char::from(second)]);
            let mut encoded = String::new();
            write!(SingleLine::new(&mut encoded), "{input}").unwrap();
            assert_eq!(decode(&encoded).as_deref(), Some(input.as_str()));
        }
    }
}

#[derive(Debug)]
struct Refuse<'a> {
    text: &'a str,
}

impl fmt::Write for Refuse<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text == self.text {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

#[test]
fn writer_errors_propagate_from_plain_and_escaped_segments() {
    let mut plain = Refuse { text: "body" };
    assert!(write!(SingleLine::new(&mut plain), "body").is_err());

    let mut before_escape = Refuse { text: "body" };
    assert!(write!(SingleLine::new(&mut before_escape), "body\nafter").is_err());

    let mut escaped = Refuse { text: "\\n" };
    assert!(write!(SingleLine::new(&mut escaped), "body\nafter").is_err());
}

#[derive(Debug)]
struct RefuseOnce {
    at: usize,
    attempted: usize,
    output: String,
}

impl RefuseOnce {
    fn at(at: usize) -> Self {
        Self {
            at,
            attempted: 0,
            output: String::new(),
        }
    }
}

impl fmt::Write for RefuseOnce {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for character in text.chars() {
            let attempted = self.attempted;
            self.attempted += 1;
            if attempted == self.at {
                return Err(fmt::Error);
            }
            self.output.push(character);
        }
        Ok(())
    }
}

#[test]
fn completion_requires_every_prior_character_write() {
    let mut complete = String::new();
    write_record(&mut complete, "boot", "bad\nstate").unwrap();
    let expected = "[error] KERNEL PANIC on thread 'boot': bad\\nstate\n\
                    [error] panic: terminal record complete v1\n";
    let terminal = "\n[error] panic: terminal record complete v1\n";
    assert_eq!(complete.as_bytes(), expected.as_bytes());
    assert_eq!(complete.lines().count(), 2);
    assert_eq!(complete.matches(terminal.trim_start()).count(), 1);

    for refused in 0..complete.chars().count() {
        let mut writer = RefuseOnce::at(refused);
        assert!(
            write_record(&mut writer, "boot", "bad\nstate").is_err(),
            "accepted refused character {refused}"
        );
        assert!(
            !writer.output.contains(terminal),
            "completed after refused character {refused}: {:?}",
            writer.output
        );
    }

    let mut writer = RefuseOnce::at(usize::MAX);
    write_record(&mut writer, "boot", "bad\nstate").unwrap();
    assert_eq!(writer.output.as_bytes(), expected.as_bytes());
}
