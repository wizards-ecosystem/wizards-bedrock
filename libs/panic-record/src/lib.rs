#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt::{self, Write as _};

pub const OPEN_PREFIX: &str = "[error] KERNEL PANIC on thread '";

const COMPLETE_MARKER: &str = "panic: terminal record complete v1";

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
                self.writer.write_str(&text[start..index])?;
            }
            self.writer.write_str(escaped)?;
            start = index + character.len_utf8();
        }
        if start != text.len() {
            self.writer.write_str(&text[start..])?;
        }
        Ok(())
    }
}

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
