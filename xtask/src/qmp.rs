use core::time::Duration;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail, ensure};

use crate::json::{JsonValue, json_string, parse_json};

const MAX_MESSAGE_BYTES: u64 = 1024 * 1024;
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const ACCEPT_POLL: Duration = Duration::from_millis(20);

#[derive(PartialEq, Eq, Debug)]
enum Message {
    Greeting,
    Return {
        id: Option<u64>,
        value: JsonValue,
    },
    Error {
        id: Option<u64>,
        description: String,
    },
}

#[derive(Debug)]
pub(crate) struct Qmp {
    writer: TcpStream,
    messages: Receiver<Result<Message>>,
    next_id: u64,
}

impl Qmp {
    // QEMU connects to a listener xtask bound itself, so no other emulator can answer.
    #[expect(
        clippy::std_instead_of_core,
        reason = "core::io::ErrorKind is unstable"
    )]
    pub(crate) fn accept(
        listener: &TcpListener,
        child: &mut Child,
        deadline: Instant,
    ) -> Result<Self> {
        listener
            .set_nonblocking(true)
            .context("polling the QMP listener")?;
        let writer = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error).context("accepting QEMU's QMP connection"),
            }
            if let Some(status) = child.try_wait().context("waiting for QEMU")? {
                bail!("QEMU exited before it connected to QMP ({status})");
            }
            ensure!(Instant::now() < deadline, "QEMU did not connect to QMP");
            std::thread::sleep(ACCEPT_POLL);
        };
        writer
            .set_nonblocking(false)
            .and_then(|()| writer.set_write_timeout(Some(REPLY_TIMEOUT)))
            .context("configuring the QMP connection")?;
        let reader = writer.try_clone().context("sharing the QMP connection")?;
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || pump(reader, &sender));
        let mut qmp = Self {
            writer,
            messages,
            next_id: 0,
        };
        match qmp.receive(deadline)? {
            Message::Greeting => {}
            other => bail!("QMP opened with {other:?} instead of its greeting"),
        }
        qmp.command("qmp_capabilities", None)
            .context("negotiating QMP capabilities")?;
        Ok(qmp)
    }

    // Explicit key down and up, because the kernel asserts both halves of the sequence.
    pub(crate) fn inject_key(&mut self, qcode: &str) -> Result<()> {
        let event = |down: &str| {
            format!(
                r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": {}}}}}}}"#,
                quote(qcode)
            )
        };
        let arguments = format!(r#"{{"events": [{}, {}]}}"#, event("true"), event("false"));
        self.command("input-send-event", Some(&arguments))
            .with_context(|| format!("injecting the {qcode} key"))
            .map(drop)
    }

    pub(crate) fn human(&mut self, command_line: &str) -> Result<String> {
        let arguments = format!(r#"{{"command-line": {}}}"#, quote(command_line));
        let value = self
            .command("human-monitor-command", Some(&arguments))
            .with_context(|| format!("running the monitor command `{command_line}`"))?;
        json_string(&value, "human monitor reply").map(str::to_string)
    }

    fn command(&mut self, execute: &str, arguments: Option<&str>) -> Result<JsonValue> {
        self.next_id += 1;
        let id = self.next_id;
        let request = match arguments {
            Some(arguments) => {
                format!(r#"{{"execute": "{execute}", "arguments": {arguments}, "id": {id}}}"#)
            }
            None => format!(r#"{{"execute": "{execute}", "id": {id}}}"#),
        };
        writeln!(self.writer, "{request}").context("writing a QMP command")?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            match self.receive(deadline)? {
                Message::Return {
                    id: Some(answered),
                    value,
                } if answered == id => return Ok(value),
                Message::Error {
                    id: Some(answered),
                    description,
                } if answered == id => bail!("QMP refused `{execute}`: {description}"),
                _ => {}
            }
        }
    }

    fn receive(&self, deadline: Instant) -> Result<Message> {
        let wait = deadline.saturating_duration_since(Instant::now());
        match self.messages.recv_timeout(wait) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => bail!("QMP did not answer in time"),
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("QMP closed its connection"),
        }
    }
}

fn pump(output: impl Read, sender: &Sender<Result<Message>>) {
    let mut reader = BufReader::new(output);
    let mut line = Vec::new();
    loop {
        line.clear();
        let message = match (&mut reader)
            .take(MAX_MESSAGE_BYTES)
            .read_until(b'\n', &mut line)
        {
            Ok(0) => return,
            Ok(_) if !line.ends_with(b"\n") => Some(Err(anyhow!("QMP sent a truncated message"))),
            Ok(_) => parse_message(&line).transpose(),
            Err(error) => Some(Err(error).context("reading QMP output")),
        };
        let Some(message) = message else {
            continue;
        };
        let failed = message.is_err();
        if sender.send(message).is_err() || failed {
            return;
        }
    }
}

// Events are dropped, and a message carrying both an error and a return reads as the error.
fn parse_message(line: &[u8]) -> Result<Option<Message>> {
    let JsonValue::Object(mut object) = parse_json(line, "QMP message")? else {
        bail!("QMP sent a message that is not an object");
    };
    if object.contains_key("event") {
        return Ok(None);
    }
    if object.contains_key("QMP") {
        return Ok(Some(Message::Greeting));
    }
    let id = match object.remove("id") {
        Some(JsonValue::Number(id)) => Some(id.parse().context("QMP sent a malformed id")?),
        Some(_) => bail!("QMP reply names a non-numeric id"),
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
    let value = object
        .remove("return")
        .context("QMP sent a message that is neither a reply nor an event")?;
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
