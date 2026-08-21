//! A hand-rolled QMP client, just big enough to press a key.
//!
//! QMP is QEMU's JSON control protocol. The harness needs three exchanges —
//! read the greeting, negotiate capabilities, send one `input-send-event` —
//! and all of them are fixed strings, so this is written by hand on the
//! `sha256.rs` precedent rather than costing a JSON dependency. Replies are
//! classified by token scanning, which is sufficient because the response
//! set is tiny and the end-to-end verdict lives in the kernel's own check:
//! this code only has to report causes, never to judge success.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// The longest any single read may take. QEMU answers in microseconds; a
/// deadline this long only ever expires on something already broken.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// One negotiated QMP connection.
pub struct Qmp {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Qmp {
    /// Negotiate on a freshly accepted stream: greeting in, capabilities out.
    ///
    /// The stream arrives from a non-blocking listener and is switched to
    /// blocking-with-deadline here, so every later exchange is bounded.
    pub fn handshake(stream: TcpStream) -> Result<Self> {
        stream
            .set_nonblocking(false)
            .context("switching the QMP stream to blocking")?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .context("setting the QMP read deadline")?;
        let reader = BufReader::new(stream.try_clone().context("cloning the QMP stream")?);
        let mut qmp = Self { stream, reader };

        let greeting = qmp.read_line().context("reading the QMP greeting")?;
        if !greeting.contains(r#""QMP""#) {
            bail!("unexpected QMP greeting: {}", greeting.trim());
        }
        qmp.command(CAPABILITIES)
            .context("negotiating capabilities")?;
        Ok(qmp)
    }

    /// Press and release the key QEMU names `qcode`, as one command.
    pub fn inject_key(&mut self, qcode: &str) -> Result<()> {
        self.command(&key_command(qcode))
            .with_context(|| format!("injecting the {qcode} key"))
    }

    /// Send one command and require its `return`, skipping unsolicited
    /// event lines in between.
    fn command(&mut self, json: &str) -> Result<()> {
        self.stream
            .write_all(json.as_bytes())
            .and_then(|()| self.stream.write_all(b"\n"))
            .context("writing a QMP command")?;
        loop {
            let line = self.read_line()?;
            match classify(&line) {
                Reply::Return => return Ok(()),
                Reply::Error => bail!("QMP refused: {}", line.trim()),
                Reply::Other => {}
            }
        }
    }

    fn read_line(&mut self) -> Result<String> {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .context("reading from QMP")?;
        if read == 0 {
            bail!("QMP connection closed");
        }
        Ok(line)
    }
}

/// The capabilities negotiation that ends the greeting phase.
const CAPABILITIES: &str = r#"{"execute": "qmp_capabilities"}"#;

/// One key transition inside an `input-send-event`.
fn key_event(qcode: &str, down: bool) -> String {
    format!(
        r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": "{qcode}"}}}}}}"#
    )
}

/// One command that presses and then releases `qcode`.
///
/// Explicit down and up rather than the monitor's `sendkey`, whose automatic
/// release fires on QEMU's schedule instead of ours — the kernel asserts
/// both halves of the sequence, so both are stated.
fn key_command(qcode: &str) -> String {
    format!(
        r#"{{"execute": "input-send-event", "arguments": {{"events": [{}, {}]}}}}"#,
        key_event(qcode, true),
        key_event(qcode, false)
    )
}

/// What one line from QEMU amounts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reply {
    /// A command succeeded.
    Return,
    /// A command was refused.
    Error,
    /// The greeting, or an unsolicited event — `SHUTDOWN` and friends arrive
    /// on this socket when the exit device fires, and must be skipped, not
    /// misread as a verdict.
    Other,
}

fn classify(line: &str) -> Reply {
    if line.contains(r#""QMP""#) || line.contains(r#""event""#) {
        Reply::Other
    } else if line.contains(r#""return""#) {
        Reply::Return
    } else if line.contains(r#""error""#) {
        Reply::Error
    } else {
        Reply::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capabilities_command_is_what_qmp_expects() {
        assert_eq!(CAPABILITIES, r#"{"execute": "qmp_capabilities"}"#);
    }

    #[test]
    fn a_key_command_presses_then_releases() {
        let command = key_command("a");

        let down = command.find(r#""down": true"#).expect("no press");
        let up = command.find(r#""down": false"#).expect("no release");
        assert!(down < up, "release before press");
        assert!(command.contains(r#""execute": "input-send-event""#));
        assert_eq!(command.matches(r#""data": "a""#).count(), 2);
    }

    #[test]
    fn a_return_line_is_success() {
        assert_eq!(classify(r#"{"return": {}}"#), Reply::Return);
    }

    #[test]
    fn an_error_line_is_failure() {
        assert_eq!(
            classify(r#"{"error": {"class": "GenericError", "desc": "no"}}"#),
            Reply::Error
        );
    }

    /// A real shutdown event, which mentions no verdict token — and would
    /// still be an event even if it did, because the event key wins.
    #[test]
    fn an_event_line_is_neither() {
        let line = r#"{"timestamp": {"seconds": 1, "microseconds": 2}, "event": "SHUTDOWN", "data": {"guest": true, "reason": "guest-shutdown"}}"#;
        assert_eq!(classify(line), Reply::Other);
    }

    #[test]
    fn the_greeting_is_not_a_reply() {
        let line = r#"{"QMP": {"version": {"qemu": {"micro": 0, "minor": 2, "major": 9}}, "capabilities": ["oob"]}}"#;
        assert_eq!(classify(line), Reply::Other);
    }
}
