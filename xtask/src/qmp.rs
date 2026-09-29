//! QMP over the emulator's private standard streams (D-346).
//!
//! QEMU speaks QMP on the stdin and stdout pipes of the child the harness
//! spawned, so no other local process can reach the control channel. One pump
//! thread drains QEMU's output for the whole run and forwards each bounded
//! message; every command carries an `id`, a deadline and a cancellation flag,
//! and only the reply naming that `id` can answer it.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::json::{JsonValue, json_string, parse_json};

/// The longest message QEMU may send; `info registers` is about 1.3 KiB.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Messages QEMU may queue before the harness reads them. One command is
/// outstanding at a time, so a well-formed peer never needs more than two.
const PENDING_MESSAGES: usize = 4;

/// How often a waiting command rechecks its deadline and cancellation.
const POLL: Duration = Duration::from_millis(20);

/// A flag that is never raised, for exchanges nothing else cancels.
pub static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);

static NEXT_COMMAND: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, PartialEq, Eq)]
enum Message {
    Greeting,
    Return {
        id: Option<String>,
        value: JsonValue,
    },
    Error {
        id: Option<String>,
        description: String,
    },
}

/// One negotiated QMP session.
pub struct Qmp {
    input: Box<dyn Write + Send>,
    messages: Receiver<Result<Message, String>>,
}

impl Qmp {
    /// Read QEMU's greeting from `output` and negotiate capabilities.
    ///
    /// The pump thread keeps draining `output` until QEMU closes it, even
    /// after a protocol failure, so a refused session never blocks QEMU.
    pub fn negotiate(
        input: impl Write + Send + 'static,
        output: impl Read + Send + 'static,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<Self> {
        let (sender, messages) = sync_channel(PENDING_MESSAGES);
        std::thread::spawn(move || pump(output, &sender));
        let mut qmp = Self {
            input: Box::new(input),
            messages,
        };
        match qmp
            .next(deadline, cancelled)
            .context("reading the QMP greeting")?
        {
            Message::Greeting => {}
            other => bail!("QMP opened with {other:?} instead of its greeting"),
        }
        qmp.command("qmp_capabilities", None, deadline, cancelled)
            .context("negotiating QMP capabilities")?;
        Ok(qmp)
    }

    /// Press and release the key QEMU names `qcode`, as one command.
    pub fn inject_key(
        &mut self,
        qcode: &str,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        self.command(
            "input-send-event",
            Some(&key_arguments(qcode)),
            deadline,
            cancelled,
        )
        .with_context(|| format!("injecting the {qcode} key"))
        .map(drop)
    }

    /// Run one human-monitor command and return its text.
    pub fn human(
        &mut self,
        command_line: &str,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<String> {
        let arguments = format!(r#"{{"command-line": {}}}"#, quote(command_line));
        let value = self
            .command(
                "human-monitor-command",
                Some(&arguments),
                deadline,
                cancelled,
            )
            .with_context(|| format!("running the monitor command `{command_line}`"))?;
        json_string(&value, "human monitor reply").map(str::to_string)
    }

    fn command(
        &mut self,
        execute: &str,
        arguments: Option<&str>,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<JsonValue> {
        let id = format!("xtask-{}", NEXT_COMMAND.fetch_add(1, Ordering::Relaxed));
        let request = match arguments {
            Some(arguments) => {
                format!(r#"{{"execute": "{execute}", "arguments": {arguments}, "id": "{id}"}}"#)
            }
            None => format!(r#"{{"execute": "{execute}", "id": "{id}"}}"#),
        };
        self.input
            .write_all(request.as_bytes())
            .and_then(|()| self.input.write_all(b"\n"))
            .and_then(|()| self.input.flush())
            .context("writing a QMP command")?;
        match self.next(deadline, cancelled)? {
            Message::Return {
                id: Some(answered),
                value,
            } if answered == id => Ok(value),
            Message::Error {
                id: Some(answered),
                description,
            } if answered == id => bail!("QMP refused `{execute}`: {description}"),
            other => bail!("QMP answered `{execute}` with {other:?}"),
        }
    }

    fn next(&self, deadline: Instant, cancelled: &AtomicBool) -> Result<Message> {
        loop {
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "the QMP exchange was cancelled"
            );
            let now = Instant::now();
            ensure!(now < deadline, "QMP did not answer before its deadline");
            match self.messages.recv_timeout(POLL.min(deadline - now)) {
                Ok(Ok(message)) => return Ok(message),
                Ok(Err(error)) => bail!("{error}"),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => bail!("QMP closed its output"),
            }
        }
    }
}

/// Forward each bounded message, then drain until QEMU closes its output.
fn pump(output: impl Read, sender: &SyncSender<Result<Message, String>>) {
    let mut reader = BufReader::new(output);
    let mut failed = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = match (&mut reader)
            .take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(read) => read,
            Err(error) => {
                let _ = sender.try_send(Err(format!("reading QMP output: {error}")));
                return;
            }
        };
        if read == 0 {
            return;
        }
        if failed {
            continue;
        }
        let message = if line.len() > MAX_MESSAGE_BYTES {
            Err(format!(
                "QMP sent a message longer than {MAX_MESSAGE_BYTES} bytes"
            ))
        } else {
            classify(&line)
        };
        let delivered = match message {
            Ok(None) => continue,
            Ok(Some(message)) => sender.try_send(Ok(message)),
            Err(error) => {
                failed = true;
                sender.try_send(Err(error))
            }
        };
        if let Err(refused) = delivered {
            failed = true;
            if let TrySendError::Full(_) = refused {
                let _ = sender.try_send(Err("QMP sent messages nobody requested".to_string()));
            }
        }
    }
}

/// Events are dropped; the greeting and replies are the only messages kept.
/// A message carrying both an error and a return reads as the error.
fn classify(line: &[u8]) -> Result<Option<Message>, String> {
    let value = parse_json(line, "QMP message").map_err(|error| format!("{error:#}"))?;
    let JsonValue::Object(mut object) = value else {
        return Err("QMP sent a message that is not an object".to_string());
    };
    if object.contains_key("event") {
        return Ok(None);
    }
    if object.contains_key("QMP") {
        return Ok(Some(Message::Greeting));
    }
    let id = match object.remove("id") {
        Some(JsonValue::String(id)) => Some(id),
        Some(_) => return Err("QMP reply names a non-string id".to_string()),
        None => None,
    };
    if let Some(error) = object.remove("error") {
        let description = match error {
            JsonValue::Object(mut error) => match error.remove("desc") {
                Some(JsonValue::String(description)) => description,
                _ => "an undescribed error".to_string(),
            },
            _ => "an undescribed error".to_string(),
        };
        return Ok(Some(Message::Error { id, description }));
    }
    let Some(value) = object.remove("return") else {
        return Err("QMP sent a message that is neither a reply nor an event".to_string());
    };
    Ok(Some(Message::Return { id, value }))
}

fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            character if u32::from(character) < 0x20 => {
                quoted.push_str(&format!("\\u{:04x}", u32::from(character)));
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

/// One key transition inside an `input-send-event`.
fn key_event(qcode: &str, down: bool) -> String {
    format!(
        r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": {}}}}}}}"#,
        quote(qcode)
    )
}

/// Press and then release `qcode`.
///
/// Explicit down and up rather than the monitor's `sendkey`, whose automatic
/// release fires on QEMU's schedule instead of ours; the kernel asserts both
/// halves of the sequence, so both are stated.
fn key_arguments(qcode: &str) -> String {
    format!(
        r#"{{"events": [{}, {}]}}"#,
        key_event(qcode, true),
        key_event(qcode, false)
    )
}

#[cfg(test)]
mod tests {
    use std::io::{PipeReader, PipeWriter};
    use std::thread::JoinHandle;

    use super::*;

    const GREETING: &str = r#"{"QMP": {"version": {"qemu": {"micro": 50, "minor": 0, "major": 11}}, "capabilities": ["oob"]}}"#;

    /// A scripted emulator: `answer` maps each command to the lines it sends.
    fn fake_qemu(
        opening: Vec<String>,
        mut answer: impl FnMut(&str, &str) -> Vec<String> + Send + 'static,
    ) -> (PipeWriter, PipeReader, JoinHandle<bool>) {
        let (commands, harness_input) = std::io::pipe().unwrap();
        let (harness_output, mut replies) = std::io::pipe().unwrap();
        let emulator = std::thread::spawn(move || {
            for line in opening {
                if writeln!(replies, "{line}").is_err() {
                    return false;
                }
            }
            for command in BufReader::new(commands).lines() {
                let Ok(command) = command else { return true };
                let JsonValue::Object(request) = parse_json(command.as_bytes(), "request").unwrap()
                else {
                    panic!("request is not an object");
                };
                let execute = json_string(&request["execute"], "execute")
                    .unwrap()
                    .to_string();
                let id = json_string(&request["id"], "id").unwrap().to_string();
                for line in answer(&execute, &id) {
                    if writeln!(replies, "{line}").is_err() {
                        return false;
                    }
                }
            }
            true
        });
        (harness_input, harness_output, emulator)
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    #[test]
    fn only_the_reply_naming_a_command_can_answer_it() {
        let (input, output, emulator) =
            fake_qemu(vec![GREETING.to_string()], |execute, id| match execute {
                "qmp_capabilities" => vec![format!(r#"{{"return": {{}}, "id": "{id}"}}"#)],
                _ => vec![
                    r#"{"event": "RTC_CHANGE", "data": {"offset": 0}}"#.to_string(),
                    format!(
                        r#"{{"return": "IRQ statistics for ioapic:\r\n 1: 7\r\n", "id": "{id}"}}"#
                    ),
                ],
            });
        let mut qmp = Qmp::negotiate(input, output, soon(), &NOT_CANCELLED).unwrap();
        assert_eq!(
            qmp.human("info irq", soon(), &NOT_CANCELLED).unwrap(),
            "IRQ statistics for ioapic:\r\n 1: 7\r\n"
        );
        drop(qmp);
        assert!(emulator.join().unwrap(), "the harness stopped reading QEMU");

        let (input, output, emulator) =
            fake_qemu(vec![GREETING.to_string()], |execute, id| match execute {
                "qmp_capabilities" => vec![format!(r#"{{"return": {{}}, "id": "{id}"}}"#)],
                _ => vec![r#"{"return": "forged", "id": "xtask-0"}"#.to_string()],
            });
        let mut qmp = Qmp::negotiate(input, output, soon(), &NOT_CANCELLED).unwrap();
        let error = qmp.human("info irq", soon(), &NOT_CANCELLED).unwrap_err();
        assert!(
            format!("{error:#}").contains("answered `human-monitor-command`"),
            "{error:#}"
        );
        drop(qmp);
        assert!(emulator.join().unwrap(), "the harness stopped reading QEMU");
    }

    #[test]
    fn a_refusal_names_the_command_and_wins_over_a_return() {
        let (input, output, emulator) = fake_qemu(vec![GREETING.to_string()], |execute, id| {
            match execute {
                "qmp_capabilities" => vec![format!(r#"{{"return": {{}}, "id": "{id}"}}"#)],
                _ => vec![format!(
                    r#"{{"return": {{}}, "error": {{"class": "GenericError", "desc": "no such key"}}, "id": "{id}"}}"#
                )],
            }
        });
        let mut qmp = Qmp::negotiate(input, output, soon(), &NOT_CANCELLED).unwrap();
        let error = qmp.inject_key("a", soon(), &NOT_CANCELLED).unwrap_err();
        assert!(
            format!("{error:#}").contains("QMP refused `input-send-event`: no such key"),
            "{error:#}"
        );
        drop(qmp);
        assert!(emulator.join().unwrap(), "the harness stopped reading QEMU");
    }

    #[test]
    fn an_oversized_message_is_refused_while_output_keeps_draining() {
        let oversized = "x".repeat(MAX_MESSAGE_BYTES + 1);
        let (input, output, emulator) = fake_qemu(
            vec![oversized, "y".repeat(4 * MAX_MESSAGE_BYTES)],
            |_, _| Vec::new(),
        );
        let error = Qmp::negotiate(input, output, soon(), &NOT_CANCELLED)
            .err()
            .unwrap();
        assert!(
            format!("{error:#}").contains(&format!("longer than {MAX_MESSAGE_BYTES} bytes")),
            "{error:#}"
        );
        assert!(emulator.join().unwrap(), "the harness stopped reading QEMU");
    }

    #[test]
    fn a_silent_peer_ends_at_the_deadline() {
        let (input, output, _emulator) = fake_qemu(vec![GREETING.to_string()], |_, _| Vec::new());
        let started = Instant::now();
        let error = Qmp::negotiate(
            input,
            output,
            Instant::now() + Duration::from_millis(200),
            &NOT_CANCELLED,
        )
        .err()
        .unwrap();
        assert!(
            format!("{error:#}").contains("before its deadline"),
            "{error:#}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_cancelled_exchange_returns_before_its_deadline() {
        let (input, output, _emulator) = fake_qemu(Vec::new(), |_, _| Vec::new());
        let cancelled = AtomicBool::new(true);
        let started = Instant::now();
        let error = Qmp::negotiate(input, output, soon(), &cancelled)
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("cancelled"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn replies_nobody_requested_are_refused() {
        let mut opening = vec![GREETING.to_string()];
        opening.extend((0..8).map(|_| r#"{"return": {}}"#.to_string()));
        let (input, output, _emulator) = fake_qemu(opening, |_, _| Vec::new());
        std::thread::sleep(Duration::from_millis(200));
        let error = Qmp::negotiate(input, output, soon(), &NOT_CANCELLED)
            .err()
            .unwrap();
        assert!(
            format!("{error:#}").contains("answered `qmp_capabilities`"),
            "{error:#}"
        );
    }

    #[test]
    fn a_key_is_pressed_then_released() {
        let arguments = key_arguments("a");
        let down = arguments.find(r#""down": true"#).expect("no press");
        let up = arguments.find(r#""down": false"#).expect("no release");
        assert!(down < up, "release before press");
        assert_eq!(arguments.matches(r#""data": "a""#).count(), 2);
    }

    #[test]
    fn command_text_is_quoted_as_one_json_string() {
        assert_eq!(quote("x/8i 0x1"), r#""x/8i 0x1""#);
        assert_eq!(quote("a\"b\\c\n"), r#""a\"b\\c\u000a""#);
    }
}
