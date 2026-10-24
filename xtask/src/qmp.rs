//! QMP over the emulator's private standard streams (D-346).
//!
//! QEMU speaks QMP on the stdin and stdout pipes of the child the harness
//! spawned, so no other local process can reach the control channel. One pump
//! thread drains QEMU's output for the whole run and forwards each bounded
//! message; every command carries an `id`, a deadline and a cancellation flag,
//! and only the reply naming that `id` can answer it.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;
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

/// Bound public text before JSON escaping and each complete queued request.
const MAX_COMMAND_TEXT_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// One command is outstanding; neither worker channel can accumulate a run.
const PENDING_WRITES: usize = 1;
const PENDING_WRITE_ACKS: usize = 1;

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

/// One negotiated QMP session. An incomplete exchange permanently refuses reuse.
pub struct Qmp {
    writer: CommandWriter,
    messages: Receiver<Result<Message, String>>,
    failed: bool,
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

    /// Press and release the key QEMU names `qcode`, as one command.
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

    /// Run one human-monitor command and return its text.
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
        let result = self.command_once(execute, arguments, deadline, cancelled);
        if result.is_ok() {
            self.failed = false;
        } else {
            self.writer.close();
        }
        result
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

struct WriteRequest {
    bytes: Vec<u8>,
    deadline: Instant,
    abandoned: Arc<AtomicBool>,
}

/// The caller waits with a deadline; only this one worker owns the OS writer.
/// Closing its producer does not synchronously join a possibly blocked write.
/// The machine owner must stop the child to close the peer after a refusal.
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
                let result = if request.abandoned.load(Ordering::Acquire)
                    || Instant::now() >= request.deadline
                {
                    Err("QMP writer refused an abandoned or expired request".to_string())
                } else {
                    input
                        .write_all(&request.bytes)
                        .and_then(|()| input.flush())
                        .map_err(|error| error.to_string())
                };
                let succeeded = result.is_ok();
                if completed.try_send(result).is_err() || !succeeded {
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
        let result = (|| {
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
        if result.is_err() {
            abandoned.store(true, Ordering::Release);
        }
        result
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

    struct ControlledWriter {
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
        closed: std::sync::mpsc::Sender<()>,
        block_flush: bool,
        succeed_after_release: bool,
    }

    impl ControlledWriter {
        fn blocked(&mut self) -> std::io::Result<()> {
            self.entered.send(()).unwrap();
            let _ = self.release.recv();
            if self.succeed_after_release {
                Ok(())
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "fixture peer released",
                ))
            }
        }
    }

    impl std::io::Write for ControlledWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.block_flush {
                Ok(bytes.len())
            } else {
                self.blocked().map(|()| bytes.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.block_flush {
                self.blocked()
            } else {
                Ok(())
            }
        }
    }

    impl Drop for ControlledWriter {
        fn drop(&mut self) {
            let _ = self.closed.send(());
        }
    }

    fn session_with_writer(input: impl Write + Send + 'static) -> Qmp {
        let (_, messages) = sync_channel(PENDING_MESSAGES);
        Qmp {
            writer: CommandWriter::new(input),
            messages,
            failed: false,
        }
    }

    fn blocked_write_case(block_flush: bool, cancel: bool, succeed_after_release: bool) {
        let (entered, blocked) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (closed, writer_closed) = std::sync::mpsc::channel();
        let (done, finished) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let caller_cancelled = Arc::clone(&cancelled);
        let caller = std::thread::spawn(move || {
            let mut qmp = session_with_writer(ControlledWriter {
                entered,
                release: released,
                closed,
                block_flush,
                succeed_after_release,
            });
            let deadline = Instant::now()
                + if cancel {
                    Duration::from_secs(30)
                } else {
                    Duration::from_secs(1)
                };
            let error = qmp
                .command("fixture", None, deadline, &caller_cancelled)
                .unwrap_err();
            let text = format!("{error:#}");
            done.send(text.clone()).unwrap();
            (qmp, text)
        });
        let entered_result = blocked.recv_timeout(Duration::from_secs(5));
        if cancel {
            cancelled.store(true, Ordering::Release);
        }
        // Capture the caller's result while the fake OS peer remains blocked.
        // Release on both pristine and mutated outcomes before assertions/join.
        let before_release = finished.recv_timeout(Duration::from_secs(3));
        let _ = release.send(());
        let (mut qmp, text) = caller.join().unwrap();
        let closed_before_cleanup = writer_closed.recv_timeout(Duration::from_secs(5));
        if closed_before_cleanup.is_err() {
            // A reuse/close mutant must not strand a successful writer waiting
            // for another request. Close it before making any assertions.
            qmp.writer.close();
            writer_closed.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(
            entered_result.is_ok(),
            "the fixture never reached its blocked operation"
        );
        assert!(
            before_release.is_ok(),
            "the caller waited for peer release: {text}"
        );
        let expected = if cancel {
            "QMP write was cancelled"
        } else {
            "QMP write did not finish before its deadline"
        };
        assert!(text.contains(expected), "{text}");
        let error = qmp
            .command("late", None, soon(), &NOT_CANCELLED)
            .unwrap_err();
        assert!(error.to_string().contains("cannot be reused"));
        if succeed_after_release {
            // The completed write really succeeded after the caller refused it;
            // the retry must not consume that stale successful acknowledgement.
            assert!(matches!(qmp.writer.acknowledgements.try_recv(), Ok(Ok(()))));
        }
        assert!(
            closed_before_cleanup.is_ok(),
            "the failed exchange left its producer open"
        );
    }

    #[test]
    fn blocked_qmp_writes_and_flushes_return_at_the_command_deadline() {
        blocked_write_case(false, false, false);
        blocked_write_case(true, false, false);
    }

    #[test]
    fn cancellation_interrupts_the_wait_for_a_blocked_qmp_write_or_flush() {
        blocked_write_case(false, true, false);
        blocked_write_case(true, true, false);
    }

    struct CountedWriter {
        bytes: Arc<AtomicU64>,
        wrote: Option<std::sync::mpsc::Sender<()>>,
        closed: std::sync::mpsc::Sender<()>,
    }

    impl std::io::Write for CountedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.fetch_add(bytes.len() as u64, Ordering::AcqRel);
            if let Some(wrote) = &self.wrote {
                let _ = wrote.send(());
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Drop for CountedWriter {
        fn drop(&mut self) {
            let _ = self.closed.send(());
        }
    }

    #[test]
    fn expired_or_cancelled_commands_do_not_write_and_poison_the_session() {
        for cancelled in [false, true] {
            let bytes = Arc::new(AtomicU64::new(0));
            let (closed, writer_closed) = std::sync::mpsc::channel();
            let mut qmp = session_with_writer(CountedWriter {
                bytes: Arc::clone(&bytes),
                wrote: None,
                closed,
            });
            let flag = AtomicBool::new(cancelled);
            let deadline = if cancelled { soon() } else { Instant::now() };
            let error = qmp.command("fixture", None, deadline, &flag).unwrap_err();
            let expected = if cancelled {
                "QMP write was cancelled"
            } else {
                "QMP write did not finish before its deadline"
            };
            assert!(format!("{error:#}").contains(expected));
            let repeated = qmp
                .command("retry", None, soon(), &NOT_CANCELLED)
                .unwrap_err();
            assert!(repeated.to_string().contains("cannot be reused"));
            drop(qmp);
            writer_closed.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(bytes.load(Ordering::Acquire), 0);
        }
    }

    #[test]
    fn an_incomplete_exchange_permanently_refuses_another_command() {
        blocked_write_case(false, false, true);
        let calls = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&calls);
        let (input, output, emulator) =
            fake_qemu(vec![GREETING.to_string()], move |execute, id| {
                if execute == "qmp_capabilities" {
                    vec![format!(r#"{{"return": {{}}, "id": "{id}"}}"#)]
                } else {
                    observed.fetch_add(1, Ordering::AcqRel);
                    vec![r#"{"return": "wrong command", "id": "xtask-0"}"#.to_string()]
                }
            });
        let mut qmp = Qmp::negotiate(input, output, soon(), &NOT_CANCELLED).unwrap();
        let first = qmp.human("info irq", soon(), &NOT_CANCELLED);
        let second = qmp.human("info registers", soon(), &NOT_CANCELLED);
        drop(qmp);
        assert!(emulator.join().unwrap());
        assert!(first.is_err());
        assert!(format!("{:#}", second.unwrap_err()).contains("cannot be reused"));
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    struct ImmediateWriteFailure;

    impl std::io::Write for ImmediateWriteFailure {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fixture write failed",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn writer_refusal_case(writer: CommandWriter, expected: &str) {
        let (_, messages) = sync_channel(PENDING_MESSAGES);
        let mut qmp = Qmp {
            writer,
            messages,
            failed: false,
        };
        let started = Instant::now();
        let error = qmp
            .command("fixture", None, soon(), &NOT_CANCELLED)
            .unwrap_err();
        let elapsed = started.elapsed();
        let repeated = qmp
            .command("retry", None, soon(), &NOT_CANCELLED)
            .unwrap_err();
        assert_eq!(format!("{error:#}"), expected);
        assert!(
            elapsed < Duration::from_secs(2),
            "writer refusal waited {elapsed:?}"
        );
        assert!(repeated.to_string().contains("cannot be reused"));
    }

    #[test]
    fn qmp_writer_errors_and_disconnects_refuse_without_waiting_for_the_deadline() {
        writer_refusal_case(
            CommandWriter::new(ImmediateWriteFailure),
            "writing a QMP command: fixture write failed",
        );

        // Keep the unrelated ACK sender alive so this exercises enqueue failure.
        let (requests, incoming) = sync_channel(PENDING_WRITES);
        let (completed, acknowledgements) = sync_channel(PENDING_WRITE_ACKS);
        drop(incoming);
        writer_refusal_case(
            CommandWriter {
                requests: Some(requests),
                acknowledgements,
            },
            "writing a QMP command: the QMP writer stopped",
        );
        drop(completed);

        // Keep a request slot alive but remove its ACK sender: enqueue succeeds,
        // then the acknowledgement wait must report the distinct disconnect.
        let (requests, incoming) = sync_channel(PENDING_WRITES);
        let (completed, acknowledgements) = sync_channel(PENDING_WRITE_ACKS);
        drop(completed);
        writer_refusal_case(
            CommandWriter {
                requests: Some(requests),
                acknowledgements,
            },
            "writing a QMP command: the QMP writer stopped before acknowledging its command",
        );
        drop(incoming);
    }

    #[test]
    fn successful_matched_replies_allow_the_next_command() {
        let (input, output, emulator) = fake_qemu(vec![GREETING.to_string()], |execute, id| {
            let value = if execute == "qmp_capabilities" {
                "{}"
            } else {
                "\"answer\""
            };
            vec![format!(r#"{{"return": {value}, "id": "{id}"}}"#)]
        });
        let mut qmp = Qmp::negotiate(input, output, soon(), &NOT_CANCELLED).unwrap();
        for _ in 0..2 {
            assert_eq!(
                qmp.human("info irq", soon(), &NOT_CANCELLED).unwrap(),
                "answer"
            );
        }
        drop(qmp);
        assert!(emulator.join().unwrap());
    }

    #[test]
    fn qmp_request_text_and_encoded_bytes_are_bounded_before_writing() {
        let bytes = Arc::new(AtomicU64::new(0));
        let (closed, writer_closed) = std::sync::mpsc::channel();
        let mut qmp = session_with_writer(CountedWriter {
            bytes: Arc::clone(&bytes),
            wrote: None,
            closed,
        });
        let text = "x".repeat(MAX_COMMAND_TEXT_BYTES + 1);
        let monitor = qmp.human(&text, soon(), &NOT_CANCELLED);
        let key = qmp.inject_key(&text, soon(), &NOT_CANCELLED);
        let arguments = "x".repeat(MAX_REQUEST_BYTES);
        let request = qmp.command("fixture", Some(&arguments), soon(), &NOT_CANCELLED);
        drop(qmp);
        writer_closed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(monitor.unwrap_err().to_string().contains("monitor text"));
        assert!(key.unwrap_err().to_string().contains("key text"));
        assert!(format!("{:#}", request.unwrap_err()).contains("request exceeds"));
        assert_eq!(bytes.load(Ordering::Acquire), 0);
    }

    #[test]
    fn the_qmp_writer_request_queue_has_one_bounded_slot() {
        let (entered, blocked) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (closed, writer_closed) = std::sync::mpsc::channel();
        let mut writer = CommandWriter::new(ControlledWriter {
            entered,
            release: released,
            closed,
            block_flush: false,
            succeed_after_release: false,
        });
        let request = || WriteRequest {
            bytes: b"fixture\n".to_vec(),
            deadline: soon(),
            abandoned: Arc::new(AtomicBool::new(false)),
        };
        let sender = writer.requests.as_ref().unwrap();
        assert!(sender.try_send(request()).is_ok());
        let entered_result = blocked.recv_timeout(Duration::from_secs(5));
        let queued = sender.try_send(request());
        let overflow = sender.try_send(request());
        let _ = release.send(());
        writer.close();
        writer_closed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(entered_result.is_ok());
        assert!(
            queued.is_ok(),
            "the one permitted pending slot was unavailable"
        );
        assert!(matches!(overflow, Err(TrySendError::Full(_))));
    }

    #[test]
    fn queued_expired_or_abandoned_requests_never_reach_the_os_writer() {
        for abandoned in [false, true] {
            let bytes = Arc::new(AtomicU64::new(0));
            let (closed, writer_closed) = std::sync::mpsc::channel();
            let mut writer = CommandWriter::new(CountedWriter {
                bytes: Arc::clone(&bytes),
                wrote: None,
                closed,
            });
            let request = WriteRequest {
                bytes: b"stale\n".to_vec(),
                deadline: if abandoned { soon() } else { Instant::now() },
                abandoned: Arc::new(AtomicBool::new(abandoned)),
            };
            let sent = writer.requests.as_ref().unwrap().try_send(request);
            let result = writer.acknowledgements.recv_timeout(Duration::from_secs(5));
            writer.close();
            writer_closed.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(sent.is_ok());
            assert!(
                matches!(result, Ok(Err(ref message)) if message.contains("abandoned or expired"))
            );
            assert_eq!(bytes.load(Ordering::Acquire), 0);
        }
    }

    #[test]
    fn a_full_qmp_write_ack_queue_stops_the_writer_without_blocking() {
        let bytes = Arc::new(AtomicU64::new(0));
        let (wrote, written) = std::sync::mpsc::channel();
        let (closed, writer_closed) = std::sync::mpsc::channel();
        let mut writer = CommandWriter::new(CountedWriter {
            bytes: Arc::clone(&bytes),
            wrote: Some(wrote),
            closed,
        });
        let request = || WriteRequest {
            bytes: b"bounded\n".to_vec(),
            deadline: Instant::now() + Duration::from_secs(30),
            abandoned: Arc::new(AtomicBool::new(false)),
        };
        let first = writer.requests.as_ref().unwrap().try_send(request());
        let first_written = written.recv_timeout(Duration::from_secs(5));
        let second = writer.requests.as_ref().unwrap().try_send(request());
        // Leave the first acknowledgement queued. The second cannot block its
        // producer or expand the queue; it ends the worker on refusal.
        let closed_before_producer = writer_closed.recv_timeout(Duration::from_secs(5));
        if closed_before_producer.is_err() {
            // Release a deliberately mutated blocking ACK send before joining
            // the fixture's closure acknowledgement and making assertions.
            let _ = writer.acknowledgements.try_recv();
        }
        writer.close();
        if closed_before_producer.is_err() {
            writer_closed.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(first.is_ok() && first_written.is_ok() && second.is_ok());
        assert!(
            closed_before_producer.is_ok(),
            "the acknowledgement queue blocked or grew"
        );
        assert_eq!(bytes.load(Ordering::Acquire), 2 * b"bounded\n".len() as u64);
        assert!(matches!(writer.acknowledgements.try_recv(), Ok(Ok(()))));
        // Input Drop can signal before the worker's separate ACK sender drops.
        // Wait for that destruction after consuming ACK1, with a finite bound.
        assert!(matches!(
            writer.acknowledgements.recv_timeout(Duration::from_secs(5)),
            Err(RecvTimeoutError::Disconnected)
        ));
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
