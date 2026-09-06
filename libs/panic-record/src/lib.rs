//! Allocation-free framing for a terminal kernel panic record.
//!
//! A panic body is arbitrary formatted text, so it can contain the physical
//! line breaks used to frame the record that follows it. [`write_record`]
//! makes those breaks and their escape introducer visible, and publishes
//! completion only after every prior write succeeds.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt::{self, Write as _};

/// The exact physical prefix of a kernel panic body.
pub const OPEN_PREFIX: &str = "[error] KERNEL PANIC on thread '";

/// The message emitted after a complete single-line panic body.
pub const COMPLETE_MARKER: &str = "panic: terminal record complete v1";

#[derive(Debug)]
struct SingleLine<'a, W: fmt::Write> {
    writer: &'a mut W,
}

impl<'a, W: fmt::Write> SingleLine<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self { writer }
    }
}

impl<W: fmt::Write> fmt::Write for SingleLine<'_, W> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let mut start = 0;
        for (index, character) in text.char_indices() {
            let escaped = match character {
                '\\' => Some("\\\\"),
                '\r' => Some("\\r"),
                '\n' => Some("\\n"),
                _ => None,
            };
            let Some(escaped) = escaped else {
                continue;
            };
            if start != index {
                core::fmt::Write::write_str(&mut *self.writer, &text[start..index])?;
            }
            core::fmt::Write::write_str(&mut *self.writer, escaped)?;
            start = index + character.len_utf8();
        }
        if start != text.len() {
            core::fmt::Write::write_str(&mut *self.writer, &text[start..])?;
        }
        Ok(())
    }
}

/// Write one complete terminal panic record.
///
/// The completion line is withheld unless the body and its physical newline
/// were both accepted by `writer`.
///
/// # Examples
///
/// ```
/// use panic_record::{COMPLETE_MARKER, write_record};
///
/// let mut output = String::new();
/// write_record(&mut output, "boot", "bad\nstate").unwrap();
/// assert_eq!(
///     output,
///     format!(
///         "[error] KERNEL PANIC on thread 'boot': bad\\nstate\n[error] {COMPLETE_MARKER}\n"
///     )
/// );
/// ```
///
/// # Errors
///
/// Returns [`fmt::Error`] after the first refused body, newline, or completion
/// write.
pub fn write_record(
    writer: &mut impl fmt::Write,
    thread: &str,
    body: impl fmt::Display,
) -> fmt::Result {
    {
        let mut record = SingleLine::new(&mut *writer);
        write!(record, "{OPEN_PREFIX}{thread}': {body}")?;
    }
    writer.write_char('\n')?;
    writeln!(writer, "[error] {COMPLETE_MARKER}")
}

#[cfg(test)]
mod tests {
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
}
