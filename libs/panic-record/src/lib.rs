//! Allocation-free panic record framing for a hostile body.
//! Its line breaks and backslashes are escaped so it cannot forge the framing.

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
struct SingleLine<'writer, W: fmt::Write> {
    writer: &'writer mut W,
}

impl<'writer, W: fmt::Write> SingleLine<'writer, W> {
    fn new(writer: &'writer mut W) -> Self {
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

/// Write one terminal panic record.
/// The completion line is withheld unless `writer` accepted every earlier write.
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
mod tests;
