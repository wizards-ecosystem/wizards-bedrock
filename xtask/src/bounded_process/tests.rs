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
