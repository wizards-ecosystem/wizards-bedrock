//! QMP over the stdin and stdout pipes of the QEMU child the harness spawned, so no other
//! local process can reach the control channel.
//! One pump thread drains QEMU's output for the whole run and forwards each bounded message.
//! Every command carries an `id`, a deadline and a cancellation flag, and only the reply naming
//! that `id` can answer it.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::json::{JsonValue, json_string, parse_json};

/// The longest message QEMU may send; `info registers` is about 1.3 KiB.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// One command is outstanding at a time, so a well-formed peer never queues more than two.
const PENDING_MESSAGES: usize = 4;

/// Caller text is bounded before JSON escaping, and each queued request as a whole.
const MAX_COMMAND_TEXT_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// One command is outstanding, so neither worker channel can accumulate a run.
const PENDING_WRITES: usize = 1;
const PENDING_WRITE_ACKS: usize = 1;

/// How often a waiting command rechecks its deadline and cancellation.
const POLL: Duration = Duration::from_millis(20);

pub static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);

static NEXT_COMMAND: AtomicU64 = AtomicU64::new(1);

#[derive(PartialEq, Eq, Debug)]
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

/// An incomplete exchange permanently refuses reuse of the session.
#[derive(Debug)]
pub struct Qmp {
    writer: CommandWriter,
    messages: Receiver<Result<Message, String>>,
    failed: bool,
}

impl Qmp {
    /// The pump thread drains `output` until QEMU closes it, even after a protocol failure,
    /// so a refused session never blocks QEMU.
    pub fn negotiate(
        input: impl Write + Send + 'static,
        output: impl Read + Send + 'static,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<Self> {
        let (sender, messages) = sync_channel(PENDING_MESSAGES);
        std::thread::spawn(move || pump(output, &sender));
        let mut qmp = Self {
            writer: CommandWriter::new(input),
            messages,
            failed: false,
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

    pub fn inject_key(
        &mut self,
        qcode: &str,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        ensure!(
            qcode.len() <= MAX_COMMAND_TEXT_BYTES,
            "QMP key text exceeds its byte limit"
        );
        self.command(
            "input-send-event",
            Some(&key_arguments(qcode)),
            deadline,
            cancelled,
        )
        .with_context(|| format!("injecting the {qcode} key"))
        .map(drop)
    }

    pub fn human(
        &mut self,
        command_line: &str,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<String> {
        ensure!(
            command_line.len() <= MAX_COMMAND_TEXT_BYTES,
            "QMP monitor text exceeds its byte limit"
        );
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
        ensure!(
            !self.failed,
            "QMP session cannot be reused after an incomplete exchange"
        );
        self.failed = true;
        let reply = self.command_once(execute, arguments, deadline, cancelled);
        if reply.is_ok() {
            self.failed = false;
        } else {
            self.writer.close();
        }
        reply
    }

    fn command_once(
        &mut self,
        execute: &str,
        arguments: Option<&str>,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<JsonValue> {
        let id = format!("xtask-{}", NEXT_COMMAND.fetch_add(1, Ordering::Relaxed));
        let mut request = match arguments {
            Some(arguments) => {
                format!(r#"{{"execute": "{execute}", "arguments": {arguments}, "id": "{id}"}}"#)
            }
            None => format!(r#"{{"execute": "{execute}", "id": "{id}"}}"#),
        };
        request.push('\n');
        self.writer
            .write(request.into_bytes(), deadline, cancelled)
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

#[derive(Debug)]
struct WriteRequest {
    bytes: Vec<u8>,
    deadline: Instant,
    abandoned: Arc<AtomicBool>,
}

/// The caller waits with a deadline, and only this one worker owns the OS writer.
/// Closing its producer does not synchronously join a possibly blocked write.
/// The machine owner must stop the child to close the peer after a refusal.
#[derive(Debug)]
struct CommandWriter {
    requests: Option<SyncSender<WriteRequest>>,
    acknowledgements: Receiver<Result<(), String>>,
}

impl CommandWriter {
    fn new(mut input: impl Write + Send + 'static) -> Self {
        let (requests, incoming) = sync_channel::<WriteRequest>(PENDING_WRITES);
        let (completed, acknowledgements) = sync_channel(PENDING_WRITE_ACKS);
        std::thread::spawn(move || {
            while let Ok(request) = incoming.recv() {
                let written = if request.abandoned.load(Ordering::Acquire)
                    || Instant::now() >= request.deadline
                {
                    Err("QMP writer refused an abandoned or expired request".to_string())
                } else {
                    input
                        .write_all(&request.bytes)
                        .and_then(|()| input.flush())
                        .map_err(|error| error.to_string())
                };
                let succeeded = written.is_ok();
                if completed.try_send(written).is_err() || !succeeded {
                    break;
                }
            }
        });
        Self {
            requests: Some(requests),
            acknowledgements,
        }
    }

    fn close(&mut self) {
        self.requests = None;
    }

    fn write(&self, bytes: Vec<u8>, deadline: Instant, cancelled: &AtomicBool) -> Result<()> {
        ensure!(
            bytes.len() <= MAX_REQUEST_BYTES,
            "QMP request exceeds its byte limit"
        );
        let abandoned = Arc::new(AtomicBool::new(false));
        let mut request = WriteRequest {
            bytes,
            deadline,
            abandoned: Arc::clone(&abandoned),
        };
        let delivered = (|| {
            let sender = self.requests.as_ref().context("the QMP writer is closed")?;
            loop {
                let wait = write_poll_budget(deadline, cancelled)?;
                match sender.try_send(request) {
                    Ok(()) => break,
                    Err(TrySendError::Full(pending)) => {
                        request = pending;
                        std::thread::sleep(wait);
                    }
                    Err(TrySendError::Disconnected(_)) => bail!("the QMP writer stopped"),
                }
            }
            loop {
                let wait = write_poll_budget(deadline, cancelled)?;
                match self.acknowledgements.recv_timeout(wait) {
                    Ok(Ok(())) => return Ok(()),
                    Ok(Err(error)) => bail!("{error}"),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        bail!("the QMP writer stopped before acknowledging its command")
                    }
                }
            }
        })();
        if delivered.is_err() {
            abandoned.store(true, Ordering::Release);
        }
        delivered
    }
}

fn write_poll_budget(deadline: Instant, cancelled: &AtomicBool) -> Result<Duration> {
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "the QMP write was cancelled"
    );
    let now = Instant::now();
    ensure!(
        now < deadline,
        "QMP write did not finish before its deadline"
    );
    Ok(POLL.min(deadline - now))
}

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
            parse_message(&line)
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

/// Events are dropped, and a message carrying both an error and a return reads as the error.
fn parse_message(line: &[u8]) -> Result<Option<Message>, String> {
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

fn key_event(qcode: &str, down: bool) -> String {
    format!(
        r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": {}}}}}}}"#,
        quote(qcode)
    )
}

/// Explicit down and up rather than `sendkey`, whose automatic release fires on QEMU's schedule;
/// the kernel asserts both halves of the sequence, so both are stated.
fn key_arguments(qcode: &str) -> String {
    format!(
        r#"{{"events": [{}, {}]}}"#,
        key_event(qcode, true),
        key_event(qcode, false)
    )
}

#[cfg(test)]
mod tests;
