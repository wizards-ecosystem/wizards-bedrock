use std::cell::Cell;
use std::ffi::OsString;
use std::io::{self, Write};
use std::rc::Rc;

use super::*;
use crate::test_support::success_status;

fn expected_fixed_git_environment() -> std::collections::BTreeMap<OsString, OsString> {
    [
        ("LC_ALL", "C"),
        ("GIT_CONFIG_SYSTEM", EMPTY_GIT_CONFIG),
        ("GIT_PAGER", ""),
        ("GIT_CONFIG_GLOBAL", EMPTY_GIT_CONFIG),
        ("LANG", "C"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_ATTR_NOSYSTEM", "1"),
        ("GIT_NO_LAZY_FETCH", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_CONFIG_COUNT", "0"),
        ("GIT_NO_REPLACE_OBJECTS", "1"),
        ("GIT_LITERAL_PATHSPECS", "1"),
        ("GIT_OPTIONAL_LOCKS", "0"),
    ]
    .into_iter()
    .map(|(name, value)| (OsString::from(name), OsString::from(value)))
    .collect()
}

enum State {
    Parked,
    ExitAfterKill,
    Exited,
    NeverExits,
}

struct ControlledChild {
    state: State,
    kill_refuses: bool,
}

impl ControlledChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self.state {
            State::Parked => Ok(None),
            State::ExitAfterKill => {
                self.state = State::Exited;
                Ok(None)
            }
            State::Exited => Ok(Some(success_status())),
            State::NeverExits => Ok(None),
        }
    }

    fn kill(&mut self) -> io::Result<()> {
        if self.kill_refuses {
            if matches!(self.state, State::ExitAfterKill) {
                self.state = State::Exited;
            }
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected kill refusal",
            ))
        } else {
            if matches!(self.state, State::Parked) {
                self.state = State::ExitAfterKill;
            }
            Ok(())
        }
    }
}

struct PollFailureChild {
    killed: Rc<Cell<bool>>,
}

struct PollObservedChild {
    child: ControlledChild,
    polls: Rc<Cell<usize>>,
}

impl PollObservedChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.polls.set(self.polls.get() + 1);
        ControlledChild::try_wait(&mut self.child)
    }

    fn kill(&mut self) -> io::Result<()> {
        ControlledChild::kill(&mut self.child)
    }
}

impl PollFailureChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Err(io::Error::other("injected poll failure"))
    }

    fn kill(&mut self) -> io::Result<()> {
        self.killed.set(true);
        Ok(())
    }
}

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _bytes: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("injected dialogue read failure"))
    }
}

impl BufRead for FailingReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other("injected dialogue read failure"))
    }

    fn consume(&mut self, _amount: usize) {}
}

#[test]
fn dialogue_lines_are_framed_and_counted_as_wire_bytes() {
    let mut framed = Vec::new();
    write_bounded_line(&mut framed, b"abc", 4, "fixture").unwrap();
    assert_eq!(framed, b"abc\n");

    let mut reader = BufReader::new(io::Cursor::new(b"abc\nxy"));
    let mut total = 0;
    assert_eq!(
        read_bounded_line(&mut reader, 4, 6, &mut total, "fixture").unwrap(),
        Some(b"abc\n".to_vec())
    );
    assert_eq!(
        read_bounded_line(&mut reader, 4, 6, &mut total, "fixture").unwrap(),
        Some(b"xy".to_vec())
    );
    assert_eq!(total, 6);
    assert_eq!(
        read_bounded_line(&mut reader, 4, 6, &mut total, "fixture").unwrap(),
        None
    );
}

#[test]
fn dialogue_request_framing_refuses_injection_and_oversize() {
    for request in [b"one\ntwo".as_slice(), b"one\rtwo".as_slice()] {
        let error = write_bounded_line(&mut Vec::new(), request, 32, "fixture").unwrap_err();
        assert!(format!("{error:#}").contains("raw line ending"));
    }

    let error = write_bounded_line(&mut Vec::new(), b"four", 4, "fixture").unwrap_err();
    assert!(format!("{error:#}").contains("4-byte line limit"));
}

#[test]
fn dialogue_line_and_cumulative_limits_fail_independently() {
    let mut overlong = BufReader::new(io::Cursor::new(b"abcd\n"));
    let error = read_bounded_line(&mut overlong, 4, 64, &mut 0, "fixture").unwrap_err();
    assert!(format!("{error:#}").contains("line longer than 4 bytes"));

    let mut cumulative = BufReader::new(io::Cursor::new(b"a\nb\n"));
    let mut total = 0;
    assert!(
        read_bounded_line(&mut cumulative, 2, 3, &mut total, "fixture")
            .unwrap()
            .is_some()
    );
    let error = read_bounded_line(&mut cumulative, 2, 3, &mut total, "fixture").unwrap_err();
    assert!(format!("{error:#}").contains("stdout exceeded 3 bytes"));
}

#[test]
fn dialogue_line_reader_propagates_io_and_length_overflow() {
    let error = read_bounded_line(&mut FailingReader, 32, 64, &mut 0, "fixture").unwrap_err();
    assert!(format!("{error:#}").contains("injected dialogue read failure"));

    #[cfg(target_pointer_width = "64")]
    {
        let mut reader = BufReader::new(io::Cursor::new(b"x"));
        let error =
            read_bounded_line(&mut reader, usize::MAX, usize::MAX, &mut 0, "fixture").unwrap_err();
        assert!(format!("{error:#}").contains("line limit overflowed"));
    }

    let mut reader = BufReader::new(io::Cursor::new(b"x"));
    let mut total = usize::MAX;
    let error = read_bounded_line(&mut reader, 1, usize::MAX, &mut total, "fixture").unwrap_err();
    assert!(format!("{error:#}").contains("stdout length overflowed"));
}

#[test]
fn a_successful_timeout_kill_is_reaped_without_a_blocking_wait() {
    let polls = Rc::new(Cell::new(0));
    let child = PollObservedChild {
        child: ControlledChild {
            state: State::Parked,
            kill_refuses: false,
        },
        polls: Rc::clone(&polls),
    };
    let error = poll_child_with(
        child,
        PollObservedChild::try_wait,
        PollObservedChild::kill,
        Duration::ZERO,
        "parked child",
        || Ok(()),
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("parked child timed out after 0 ms"));
    assert_eq!(polls.get(), 3, "the killed child was not reaped by polling");
}

#[test]
fn a_failed_timeout_kill_is_returned_before_a_blocking_wait() {
    let child = ControlledChild {
        state: State::Parked,
        kill_refuses: true,
    };
    let error = poll_child_with(
        child,
        ControlledChild::try_wait,
        ControlledChild::kill,
        Duration::ZERO,
        "kill-refusing child",
        || Ok(()),
    )
    .unwrap_err();
    let report = format!("{error:#}");

    assert!(report.contains("stopping timed-out kill-refusing child"));
    assert!(report.contains("injected kill refusal"));
}

#[test]
fn an_exit_during_a_failed_kill_preserves_the_timeout_classification() {
    let child = ControlledChild {
        state: State::ExitAfterKill,
        kill_refuses: true,
    };

    let error = poll_child_with(
        child,
        ControlledChild::try_wait,
        ControlledChild::kill,
        Duration::ZERO,
        "racing child",
        || Ok(()),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("racing child timed out after 0 ms"));
}

#[test]
fn post_kill_polling_observes_delayed_exit_and_refuses_no_exit() {
    let mut delayed = ControlledChild {
        state: State::ExitAfterKill,
        kill_refuses: false,
    };
    assert!(
        poll_terminated_child(
            &mut delayed,
            ControlledChild::try_wait,
            Duration::from_secs(1),
            "delayed child",
        )
        .unwrap()
        .success()
    );

    let mut stuck = ControlledChild {
        state: State::NeverExits,
        kill_refuses: false,
    };
    let error = poll_terminated_child(
        &mut stuck,
        ControlledChild::try_wait,
        Duration::ZERO,
        "stuck child",
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("did not exit within 0 ms after termination"));
}

#[test]
fn an_exited_child_never_waits_for_inherited_output_eof() {
    let child = ControlledChild {
        state: State::Exited,
        kill_refuses: false,
    };

    assert!(
        poll_child_with(
            child,
            ControlledChild::try_wait,
            ControlledChild::kill,
            Duration::from_secs(1),
            "exited child",
            || Ok(()),
        )
        .unwrap()
        .success()
    );
}

#[test]
fn a_poll_failure_stops_the_direct_child_before_returning() {
    let killed = Rc::new(Cell::new(false));
    let child = PollFailureChild {
        killed: Rc::clone(&killed),
    };

    let error = poll_child_with(
        child,
        PollFailureChild::try_wait,
        PollFailureChild::kill,
        Duration::from_secs(1),
        "unpollable child",
        || Ok(()),
    )
    .unwrap_err();

    assert!(killed.get(), "the live child was left running");
    assert!(format!("{error:#}").contains("injected poll failure"));
}

#[test]
fn a_noisy_child_is_stopped_before_collection() {
    let child = ControlledChild {
        state: State::Parked,
        kill_refuses: false,
    };
    let error = poll_child_with(
        child,
        ControlledChild::try_wait,
        ControlledChild::kill,
        Duration::from_secs(1),
        "noisy child",
        || bail!("injected capture limit"),
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("injected capture limit"));
}

#[test]
fn captured_output_cannot_grow_collection_memory_past_its_limit() {
    let (capture, stdout, stderr) = Capture::reserve().unwrap();
    drop((stdout, stderr));
    std::fs::write(&capture.stdout.path, b"four").unwrap();
    std::fs::write(&capture.stderr.path, b"two").unwrap();

    let error = collect_output(&capture, success_status(), 5).unwrap_err();
    assert!(format!("{error:#}").contains("exceeded 5 bytes"));
}

#[test]
fn a_second_stream_failure_rolls_back_the_first_capture_and_directory() {
    let parent = crate::test_support::reserve_temp_directory("wizard-capture-rollback");
    let mut stream = 0;

    let Err(error) = Capture::reserve_in(&parent, |_, _| {
        stream += 1;
        ensure!(stream != 2, "injected second-stream failure");
        Ok(())
    }) else {
        panic!("injected second-stream failure was accepted");
    };

    assert!(format!("{error:#}").contains("injected second-stream failure"));
    assert!(crate::paths::directory_entries(&parent).unwrap().is_empty());
    std::fs::remove_dir(parent).unwrap();
}

#[test]
fn capture_growth_is_refused_while_the_child_is_still_polled() {
    let (capture, stdout, stderr) = Capture::reserve().unwrap();
    drop((stdout, stderr));
    std::fs::write(&capture.stdout.path, b"four").unwrap();
    std::fs::write(&capture.stderr.path, b"two").unwrap();

    let error = capture.require_within_limit(5).unwrap_err();
    assert!(format!("{error:#}").contains("exceeded 5 bytes"));
}

#[test]
fn collection_reads_the_original_capture_handle_not_a_substituted_path() {
    let (capture, mut stdout, stderr) = Capture::reserve().unwrap();
    stdout.write_all(b"authentic child output").unwrap();
    stdout.sync_all().unwrap();
    drop((stdout, stderr));
    let substituted = capture.directory.join("substituted-output");
    std::fs::write(&substituted, b"forged path output").unwrap();

    let observed = read_limited(
        &capture.stdout.file,
        &substituted,
        MAX_CAPTURE_BYTES,
        "output",
    )
    .unwrap();

    assert_eq!(observed, b"authentic child output");
    std::fs::remove_file(substituted).unwrap();
}

const DIALOGUE_MODE: &str = "WIZARD_BOUNDED_DIALOGUE_MODE";

#[test]
#[ignore]
fn dialogue_fixture_child() {
    let Some(mode) = std::env::var_os(DIALOGUE_MODE) else {
        return;
    };
    match mode.to_string_lossy().as_ref() {
        "exchange" => {
            let input = std::io::stdin();
            let mut lines = input.lock().lines();
            for index in 1..=2 {
                let request = lines.next().unwrap().unwrap();
                println!("wizard-dialogue-reply-{index}:{request}");
                std::io::stdout().flush().unwrap();
            }
            eprintln!("wizard dialogue diagnostic");
        }
        "chatty" => {
            for _ in 0..1_000 {
                println!("tick");
                std::io::stdout().flush().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        "cumulative" => {
            for _ in 0..1_000 {
                println!("tick");
                std::io::stdout().flush().unwrap();
            }
            std::thread::sleep(Duration::from_secs(10));
        }
        "close-trailing" | "delayed-close-trailing" => {
            let delayed = mode == "delayed-close-trailing";
            if delayed {
                std::thread::sleep(Duration::from_millis(300));
            }
            let input = std::io::stdin();
            let mut lines = input.lock().lines();
            let request = lines.next().unwrap().unwrap();
            println!("wizard-dialogue-before-close:{request}");
            std::io::stdout().flush().unwrap();
            assert!(lines.next().is_none(), "dialogue input remained open");
            if delayed {
                std::thread::sleep(Duration::from_secs(2));
            }
            println!("wizard-dialogue-after-close");
            std::io::stdout().flush().unwrap();
        }
        "early-exit" => {}
        other => panic!("unknown bounded dialogue fixture mode {other}"),
    }
}

fn dialogue_fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "bounded_process::tests::dialogue_fixture_child",
            "--ignored",
            "--nocapture",
        ])
        .env(DIALOGUE_MODE, mode);
    command
}

fn dialogue_limits() -> DialogueLimits {
    DialogueLimits {
        line_bytes: 1024,
        stdout_bytes: 1024 * 1024,
        stderr_bytes: 1024 * 1024,
    }
}

fn receive_fixture_reply(dialogue: &mut LineDialogue, marker: &str) -> Result<String> {
    loop {
        let line = dialogue
            .receive_line()?
            .context("dialogue fixture exited before its reply")?;
        let line = String::from_utf8(line).context("dialogue fixture emitted non-UTF-8")?;
        if line.starts_with(marker) {
            return Ok(line);
        }
    }
}

#[test]
fn a_completed_dialogue_exchanges_lines_and_captures_stderr() {
    let captured = output_dialogue(
        dialogue_fixture("exchange"),
        Duration::from_secs(5),
        "dialogue exchange fixture",
        dialogue_limits(),
        || Ok(()),
        |dialogue| {
            dialogue.send_line(b"first")?;
            let first = receive_fixture_reply(dialogue, "wizard-dialogue-reply-1:")?;
            dialogue.send_line(b"second")?;
            let second = receive_fixture_reply(dialogue, "wizard-dialogue-reply-2:")?;
            while dialogue.receive_line()?.is_some() {}
            Ok((first, second))
        },
    )
    .unwrap();

    assert!(captured.status.success(), "{captured:?}");
    assert!(captured.value.0.contains("first"));
    assert!(captured.value.1.contains("second"));
    assert!(
        String::from_utf8(captured.stderr)
            .unwrap()
            .contains("wizard dialogue diagnostic")
    );
}

#[test]
fn dialogue_activity_does_not_reset_the_original_deadline() {
    let activity = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = std::sync::Arc::clone(&activity);
    let error = output_dialogue(
        dialogue_fixture("chatty"),
        Duration::from_secs(1),
        "chatty dialogue fixture",
        dialogue_limits(),
        || Ok(()),
        move |dialogue| {
            while dialogue.receive_line()?.is_some() {
                observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(())
        },
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("timed out after 1000 ms"));
    assert!(
        activity.load(std::sync::atomic::Ordering::Relaxed) >= 2,
        "the fixture timed out before proving repeated dialogue activity"
    );
}

#[test]
fn closing_dialogue_input_retains_stdout_through_trailing_eof() {
    let captured = output_dialogue(
        dialogue_fixture("close-trailing"),
        Duration::from_secs(5),
        "half-closed dialogue fixture",
        dialogue_limits(),
        || Ok(()),
        |dialogue| {
            dialogue.send_line(b"request")?;
            let before = receive_fixture_reply(dialogue, "wizard-dialogue-before-close:")?;
            dialogue.close_input();
            let error = dialogue.send_line(b"late request").unwrap_err();
            ensure!(
                format!("{error:#}").contains("input is closed"),
                "send-after-close returned the wrong error: {error:#}"
            );
            let after = receive_fixture_reply(dialogue, "wizard-dialogue-after-close")?;
            while dialogue.receive_line()?.is_some() {}
            Ok((before, after))
        },
    )
    .unwrap();

    assert!(captured.status.success(), "{captured:?}");
    assert!(captured.value.0.contains("request"));
    assert!(captured.value.1.contains("after-close"));
}

#[test]
fn closing_dialogue_input_does_not_reset_the_original_deadline() {
    let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = std::sync::Arc::clone(&closed);
    let error = output_dialogue(
        dialogue_fixture("delayed-close-trailing"),
        Duration::from_millis(1_500),
        "delayed half-closed dialogue fixture",
        dialogue_limits(),
        || Ok(()),
        move |dialogue| {
            dialogue.send_line(b"request")?;
            receive_fixture_reply(dialogue, "wizard-dialogue-before-close:")?;
            dialogue.close_input();
            observed.store(true, std::sync::atomic::Ordering::Relaxed);
            receive_fixture_reply(dialogue, "wizard-dialogue-after-close")
        },
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("timed out after 1500 ms"));
    assert!(
        closed.load(std::sync::atomic::Ordering::Relaxed),
        "the fixture timed out before dialogue input was half-closed"
    );
}

#[test]
fn a_dialogue_deadline_preserves_the_exact_duration() {
    let start = Instant::now();
    let timeout = Duration::from_millis(1_500);
    let deadline = dialogue_deadline(start, timeout).expect("representable dialogue deadline");

    assert_eq!(deadline.saturating_duration_since(start), timeout);
}

#[test]
fn an_early_dialogue_exit_returns_the_reader_failure() {
    let error = output_dialogue(
        dialogue_fixture("early-exit"),
        Duration::from_secs(5),
        "early dialogue fixture",
        dialogue_limits(),
        || Ok(()),
        |dialogue| -> Result<()> {
            while dialogue.receive_line()?.is_some() {}
            bail!("injected premature dialogue EOF")
        },
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("injected premature dialogue EOF"));
}

#[test]
#[ignore]
fn capture_fixture_child() {
    println!("captured stdout");
    eprintln!("captured stderr");
}

#[test]
#[ignore]
fn noisy_stderr_fixture_child() {
    std::io::stderr().write_all(&[b'x'; 4096]).unwrap();
    std::io::stderr().flush().unwrap();
    std::thread::sleep(Duration::from_secs(10));
}

const PARKED_LOCK: &str = "WIZARD_PARKED_GRANDCHILD_LOCK";
const PARENT_LINGERS: &str = "WIZARD_PARKED_PARENT_LINGERS";

/// Holds its lock file exclusively for a minute unless its job ends it.
#[test]
#[ignore]
fn parked_grandchild_fixture() {
    let Some(path) = std::env::var_os(PARKED_LOCK) else {
        return;
    };
    #[cfg(windows)]
    let _held = {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(path)
            .unwrap()
    };
    #[cfg(not(windows))]
    let _held = File::create(path).unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

/// Starts the parked grandchild unawaited, sees its lock taken, then exits or lingers as asked.
#[test]
#[ignore]
fn grandparent_fixture_child() {
    let Some(path) = std::env::var_os(PARKED_LOCK).map(PathBuf::from) else {
        return;
    };
    let mut grandchild = Command::new(std::env::current_exe().unwrap());
    grandchild
        .args([
            "--exact",
            "bounded_process::tests::parked_grandchild_fixture",
            "--ignored",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Deliberately never awaited: the grandchild must outlive this process.
    drop(grandchild.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the grandchild never took its lock"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    if std::env::var_os(PARENT_LINGERS).is_some() {
        std::thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(windows)]
fn parked_family(label: &str, lingers: bool) -> (PathBuf, PathBuf, Command) {
    let root = crate::test_support::reserve_temp_directory(&format!("wizard-parked-{label}"));
    let lock = root.join("parked.lock");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "bounded_process::tests::grandparent_fixture_child",
            "--ignored",
        ])
        .env(PARKED_LOCK, &lock);
    if lingers {
        command.env(PARENT_LINGERS, "1");
    }
    (root, lock, command)
}

/// The lock goes away only once no process holds it open.
#[cfg(windows)]
fn released(lock: &std::path::Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if std::fs::remove_file(lock).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[cfg(windows)]
#[test]
fn a_completed_command_leaves_no_parked_descendant() {
    let (root, lock, command) = parked_family("completed", false);
    let result = output(command, Duration::from_secs(60), "grandparent fixture").unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(
        released(&lock),
        "a parked grandchild outlived its completed bounded command"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn a_timed_out_command_takes_its_parked_descendant_with_it() {
    let (root, lock, command) = parked_family("timed-out", true);
    let error = output(
        command,
        Duration::from_secs(8),
        "lingering grandparent fixture",
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("timed out"), "{error:#}");
    assert!(
        released(&lock),
        "a parked grandchild outlived its timed-out bounded command"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn a_failed_dialogue_observer_takes_its_parked_descendant_with_it() {
    let (root, lock, command) = parked_family("dialogue-observer-failed", true);
    let observed_lock = lock.clone();
    let error = output_dialogue(
        command,
        Duration::from_secs(30),
        "observed grandparent dialogue fixture",
        dialogue_limits(),
        move || {
            if !observed_lock.exists() {
                return Ok(());
            }
            bail!("injected dialogue owner observation failure")
        },
        |dialogue| {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                if dialogue.receive_line()?.is_none() {
                    return Ok(());
                }
                ensure!(
                    Instant::now() < deadline,
                    "the observed dialogue did not fail before its local deadline"
                );
            }
        },
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("injected dialogue owner observation failure"),
        "{error:#}"
    );
    assert!(
        released(&lock),
        "a parked grandchild outlived its failed bounded dialogue"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore]
fn noisy_capture_fixture_child() {
    std::io::stdout().write_all(&[b'x'; 4096]).unwrap();
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(10));
}

#[test]
fn a_completed_process_is_captured_from_regular_files() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "bounded_process::tests::capture_fixture_child",
        "--ignored",
        "--nocapture",
    ]);

    let captured = output(command, Duration::from_secs(2), "capture fixture").unwrap();
    assert!(captured.status.success());
    assert!(
        String::from_utf8(captured.stdout)
            .unwrap()
            .contains("captured stdout")
    );
    assert!(
        String::from_utf8(captured.stderr)
            .unwrap()
            .contains("captured stderr")
    );
}

#[test]
fn a_completed_process_can_reduce_stdout_while_stderr_stays_bounded() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "bounded_process::tests::capture_fixture_child",
        "--ignored",
        "--nocapture",
    ]);

    let streamed = output_streamed(
        command,
        Duration::from_secs(2),
        "streaming fixture",
        64,
        |mut stdout| {
            let mut value = String::new();
            stdout.read_to_string(&mut value)?;
            Ok(value)
        },
    )
    .unwrap();

    assert!(streamed.status.success());
    assert!(streamed.value.contains("captured stdout"));
    assert!(
        String::from_utf8(streamed.stderr)
            .unwrap()
            .contains("captured stderr")
    );
}

#[test]
fn a_stream_reducer_failure_is_returned() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "bounded_process::tests::noisy_capture_fixture_child",
        "--ignored",
        "--nocapture",
    ]);

    let error = output_streamed(
        command,
        Duration::from_secs(2),
        "failing reducer fixture",
        64,
        |_stdout| -> Result<()> { bail!("injected reducer failure") },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("injected reducer failure"));
}

#[test]
fn an_overlong_dialogue_line_is_refused_while_the_child_is_live() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "bounded_process::tests::noisy_capture_fixture_child",
        "--ignored",
        "--nocapture",
    ]);
    let limits = DialogueLimits {
        line_bytes: 64,
        stdout_bytes: 1024,
        stderr_bytes: 1024,
    };

    let error = output_dialogue(
        command,
        Duration::from_secs(2),
        "overlong dialogue fixture",
        limits,
        || Ok(()),
        |dialogue| {
            while dialogue.receive_line()?.is_some() {}
            Ok(())
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("line longer than 64 bytes"));
}

#[test]
fn dialogue_cumulative_stdout_is_refused_while_child_is_live() {
    let limits = DialogueLimits {
        line_bytes: 1024,
        stdout_bytes: 128,
        stderr_bytes: 1024,
    };

    let error = output_dialogue(
        dialogue_fixture("cumulative"),
        Duration::from_secs(2),
        "cumulative dialogue fixture",
        limits,
        || Ok(()),
        |dialogue| {
            while dialogue.receive_line()?.is_some() {}
            Ok(())
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("exceeded 128 bytes"));
}

#[test]
fn dialogue_stderr_is_refused_while_the_child_is_live() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "bounded_process::tests::noisy_stderr_fixture_child",
        "--ignored",
        "--nocapture",
    ]);
    let limits = DialogueLimits {
        line_bytes: 1024,
        stdout_bytes: 1024,
        stderr_bytes: 32,
    };

    let error = output_dialogue(
        command,
        Duration::from_secs(2),
        "noisy dialogue stderr fixture",
        limits,
        || Ok(()),
        |dialogue| {
            while dialogue.receive_line()?.is_some() {}
            Ok(())
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("exceeded 32 bytes"));
}

#[test]
fn streamed_stderr_is_refused_while_the_child_is_live() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "bounded_process::tests::noisy_stderr_fixture_child",
        "--ignored",
        "--nocapture",
    ]);

    let error = output_streamed(
        command,
        Duration::from_secs(2),
        "noisy stderr fixture",
        32,
        |mut stdout| {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes)?;
            Ok(bytes)
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("exceeded 32 bytes"));
}

#[test]
fn a_caller_specific_capture_limit_is_polled_while_the_child_is_live() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "bounded_process::tests::noisy_capture_fixture_child",
        "--ignored",
        "--nocapture",
    ]);

    let error = output_with_limit(command, Duration::from_secs(2), "noisy capture fixture", 32)
        .unwrap_err();
    assert!(format!("{error:#}").contains("exceeded 32 bytes"));
}

#[test]
#[ignore]
fn isolated_git_environment_fixture_child() {
    for (name, expected) in expected_fixed_git_environment() {
        assert_eq!(std::env::var_os(&name), Some(expected), "{name:?} differs");
    }
    for name in [
        "BASH_ENV",
        "COMSPEC",
        "DYLD_INSERT_LIBRARIES",
        "ENV",
        "GIT_DIR",
        "GIT_EXTERNAL_DIFF",
        "GIT_FUTURE_INJECTION",
        "GIT_TRACE2_EVENT",
        "LD_PRELOAD",
        "PATH",
        "PROMPT_COMMAND",
        "SHELL",
        "WIZARD_GIT_FIXTURE",
    ] {
        assert!(
            std::env::var_os(name).is_none(),
            "isolated child inherited {name}"
        );
    }
}

#[test]
fn git_children_receive_only_the_fixed_environment() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "bounded_process::tests::isolated_git_environment_fixture_child",
            "--ignored",
            "--nocapture",
        ])
        .env("BASH_ENV", "attacker-controlled")
        .env("COMSPEC", "attacker-controlled")
        .env("DYLD_INSERT_LIBRARIES", "attacker-controlled")
        .env("ENV", "attacker-controlled")
        .env("GIT_DIR", "attacker-controlled")
        .env("GIT_EXTERNAL_DIFF", "attacker-controlled")
        .env("GIT_TRACE2_EVENT", "attacker-controlled")
        .env("GIT_FUTURE_INJECTION", "attacker-controlled")
        .env("LD_PRELOAD", "attacker-controlled")
        .env("PATH", "attacker-controlled")
        .env("PROMPT_COMMAND", "attacker-controlled")
        .env("SHELL", "attacker-controlled")
        .env("WIZARD_GIT_FIXTURE", "attacker-controlled");
    isolate_git_environment(&mut command);
    let environment: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_owned(),
                value
                    .expect("fixed Git environment removed a variable")
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(environment, expected_fixed_git_environment());

    let output = output_with_limit(
        command,
        Duration::from_secs(10),
        "isolated Git environment fixture",
        64 * 1024,
    )
    .unwrap();
    assert!(output.status.success(), "{output:?}");
}
