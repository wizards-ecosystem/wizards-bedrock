//! Bounded child execution without pipe-EOF waits.
//!
//! Regular-file capture prevents inherited pipe handles from extending a wait.
//! Termination gets its own bounded poll. On Windows every command runs in a
//! job object that holds each process it starts, and the whole job is
//! terminated when the command ends, times out or fails (D-347). The byte cap
//! bounds memory rather than disk retained through an unlinked capture file.

use std::fs::{File, OpenOptions};
use std::io::{Read, SeekFrom};
use std::path::PathBuf;
#[cfg(not(windows))]
use std::process::Child;
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};

const POLL_INTERVAL: Duration = Duration::from_millis(25);
const POST_KILL_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
const GIT_REDIRECT_ENVIRONMENT: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_SYSTEM",
    "GIT_DIR",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_GLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
    "GIT_INDEX_FILE",
    "GIT_LITERAL_PATHSPECS",
    "GIT_NAMESPACE",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_OBJECT_DIRECTORY",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_WORK_TREE",
];
static NEXT_CAPTURE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn isolate_git_environment(command: &mut Command) {
    for name in GIT_REDIRECT_ENVIRONMENT {
        command.env_remove(name);
    }
    command
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_OPTIONAL_LOCKS", "0");
}

/// Run one non-interactive child through its deadline and capture both streams.
///
/// Failure cleanup may consume the additional bounded post-kill interval.
pub(crate) fn output(command: Command, timeout: Duration, label: &str) -> Result<Output> {
    output_with_limit(command, timeout, label, MAX_CAPTURE_BYTES)
}

/// Run one child with a caller-specific combined stdout/stderr limit.
pub(crate) fn output_with_limit(
    mut command: Command,
    timeout: Duration,
    label: &str,
    capture_limit: usize,
) -> Result<Output> {
    let (capture, stdout, stderr) = Capture::create()?;
    command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let tree = Tree::spawn(command).with_context(|| format!("launching {label}"))?;
    let status = poll_child_with(
        tree,
        Tree::try_wait,
        Tree::terminate,
        timeout,
        label,
        || capture.require_within_limit(capture_limit),
    )?;
    collect_output(&capture, status, capture_limit)
}

#[derive(Debug)]
pub(crate) struct StreamedOutput<T> {
    pub(crate) status: ExitStatus,
    pub(crate) value: T,
    pub(crate) stderr: Vec<u8>,
}

/// Run one child while reducing stdout on a dedicated reader thread.
///
/// The consumer must bound each record before allocating it. Stderr remains a
/// separately bounded diagnostic stream. Reader errors are observed while the
/// child is live so the complete process tree is stopped before returning.
pub(crate) fn output_streamed<T, F>(
    mut command: Command,
    timeout: Duration,
    label: &str,
    stderr_limit: usize,
    consume_stdout: F,
) -> Result<StreamedOutput<T>>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T> + Send + 'static,
{
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut tree = Tree::spawn(command).with_context(|| format!("launching {label}"))?;
    let stdout = tree
        .take_stdout()
        .with_context(|| format!("{label} has no piped stdout"))?;
    let stderr = tree
        .take_stderr()
        .with_context(|| format!("{label} has no piped stderr"))?;

    let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
    let stdout_thread = std::thread::Builder::new()
        .name("wizard-bounded-stdout".to_string())
        .spawn(move || {
            let _ = stdout_sender.send(consume_stdout(stdout));
        })
        .with_context(|| format!("starting {label} stdout reducer"))?;
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    let stderr_label = label.to_string();
    let stderr_thread = std::thread::Builder::new()
        .name("wizard-bounded-stderr".to_string())
        .spawn(move || {
            let result = read_pipe_with_limit(stderr, stderr_limit, &stderr_label);
            let _ = stderr_sender.send(result);
        })
        .with_context(|| format!("starting {label} stderr reader"))?;

    let mut stdout_value = None;
    let mut stderr_value = None;
    let status = poll_child_with(
        tree,
        Tree::try_wait,
        Tree::terminate,
        timeout,
        label,
        || {
            receive_ready(
                &stdout_receiver,
                &mut stdout_value,
                &format!("{label} stdout reducer"),
            )?;
            receive_ready(
                &stderr_receiver,
                &mut stderr_value,
                &format!("{label} stderr reader"),
            )
        },
    )?;

    let value = receive_after_exit(
        &stdout_receiver,
        stdout_value,
        &format!("{label} stdout reducer"),
    )?;
    let stderr = receive_after_exit(
        &stderr_receiver,
        stderr_value,
        &format!("{label} stderr reader"),
    )?;
    join_reader(stdout_thread, &format!("{label} stdout reducer"))?;
    join_reader(stderr_thread, &format!("{label} stderr reader"))?;
    Ok(StreamedOutput {
        status,
        value,
        stderr,
    })
}

fn receive_ready<T>(
    receiver: &Receiver<Result<T>>,
    value: &mut Option<T>,
    label: &str,
) -> Result<()> {
    if value.is_some() {
        return Ok(());
    }
    match receiver.try_recv() {
        Ok(Ok(received)) => *value = Some(received),
        Ok(Err(error)) => return Err(error).with_context(|| format!("reading {label}")),
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => bail!("{label} ended without a result"),
    }
    Ok(())
}

fn receive_after_exit<T>(
    receiver: &Receiver<Result<T>>,
    value: Option<T>,
    label: &str,
) -> Result<T> {
    if let Some(value) = value {
        return Ok(value);
    }
    match receiver.recv_timeout(POST_KILL_TIMEOUT) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error).with_context(|| format!("reading {label}")),
        Err(mpsc::RecvTimeoutError::Timeout) => bail!(
            "{label} did not close within {} ms after the child tree ended",
            POST_KILL_TIMEOUT.as_millis()
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => bail!("{label} ended without a result"),
    }
}

fn read_pipe_with_limit(mut pipe: ChildStderr, limit: usize, label: &str) -> Result<Vec<u8>> {
    let maximum = u64::try_from(limit)
        .context("bounded stream limit does not fit u64")?
        .checked_add(1)
        .context("bounded stream limit overflowed")?;
    let mut bytes = Vec::new();
    pipe.by_ref()
        .take(maximum)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {label}"))?;
    ensure!(bytes.len() <= limit, "{label} exceeded {limit} bytes");
    Ok(bytes)
}

fn join_reader(thread: JoinHandle<()>, label: &str) -> Result<()> {
    thread
        .join()
        .map_err(|_| anyhow::anyhow!("{label} panicked"))
}

/// One bounded command and every process it starts. On Windows they share a
/// job object that is terminated as a whole; dropping the tree does that, so
/// no descendant outlives the command however it ends.
struct Tree {
    #[cfg(windows)]
    child: command_group::GroupChild,
    #[cfg(not(windows))]
    child: Child,
}

impl Tree {
    /// On Windows the child is created suspended, assigned to its job and
    /// only then resumed, so nothing it starts can escape the job.
    fn spawn(command: Command) -> std::io::Result<Self> {
        #[cfg(windows)]
        let child = {
            use command_group::CommandGroup;
            let mut command = command;
            command.group().spawn()?
        };
        #[cfg(not(windows))]
        let child = {
            let mut command = command;
            command.spawn()?
        };
        Ok(Self { child })
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Terminate every process in the job; elsewhere only the direct child.
    fn terminate(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    fn take_stdout(&mut self) -> Option<ChildStdout> {
        #[cfg(windows)]
        {
            self.child.inner().stdout.take()
        }
        #[cfg(not(windows))]
        {
            self.child.stdout.take()
        }
    }

    fn take_stderr(&mut self) -> Option<ChildStderr> {
        #[cfg(windows)]
        {
            self.child.inner().stderr.take()
        }
        #[cfg(not(windows))]
        {
            self.child.stderr.take()
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

fn collect_output(capture: &Capture, status: ExitStatus, capture_limit: usize) -> Result<Output> {
    capture.collect_with_limit(status, capture_limit)
}

fn poll_child_with<C, F>(
    mut child: C,
    try_wait: fn(&mut C) -> std::io::Result<Option<ExitStatus>>,
    kill: fn(&mut C) -> std::io::Result<()>,
    timeout: Duration,
    label: &str,
    mut observe: F,
) -> Result<ExitStatus>
where
    F: FnMut() -> Result<()>,
{
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("bounded child deadline overflowed")?;
    loop {
        if let Err(error) = observe() {
            return stop_after_observation_failure(child, try_wait, kill, error, label);
        }
        let status = match try_wait(&mut child) {
            Ok(status) => status,
            Err(error) => {
                let error = anyhow::anyhow!("checking {label}: {error}");
                return stop_after_observation_failure(child, try_wait, kill, error, label);
            }
        };
        if let Some(status) = status {
            return Ok(status);
        }
        let now = Instant::now();
        if now >= deadline {
            let error = anyhow::anyhow!("{label} timed out after {} ms", timeout.as_millis());
            return stop_after_failure(child, try_wait, kill, error, label, "timed-out");
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
}

fn stop_after_observation_failure<C>(
    child: C,
    try_wait: fn(&mut C) -> std::io::Result<Option<ExitStatus>>,
    kill: fn(&mut C) -> std::io::Result<()>,
    error: anyhow::Error,
    label: &str,
) -> Result<ExitStatus> {
    stop_after_failure(child, try_wait, kill, error, label, "capture-failed")
}

fn stop_after_failure<C>(
    mut child: C,
    try_wait: fn(&mut C) -> std::io::Result<Option<ExitStatus>>,
    kill: fn(&mut C) -> std::io::Result<()>,
    error: anyhow::Error,
    label: &str,
    reason: &str,
) -> Result<ExitStatus> {
    if let Err(kill_error) = kill(&mut child) {
        let exited = try_wait(&mut child)
            .with_context(|| format!("checking {label} after termination failed"))?;
        if exited.is_none() {
            return Err(anyhow::anyhow!(
                "{error:#}; stopping {reason} {label}: {kill_error}"
            ));
        }
    } else if let Err(reap) = poll_terminated_child(&mut child, try_wait, POST_KILL_TIMEOUT, label)
    {
        return Err(anyhow::anyhow!("{error:#}; {reap:#}"));
    }
    Err(error).with_context(|| format!("{reason} {label} was not accepted"))
}

fn poll_terminated_child<C>(
    child: &mut C,
    try_wait: fn(&mut C) -> std::io::Result<Option<ExitStatus>>,
    timeout: Duration,
    label: &str,
) -> Result<ExitStatus> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("post-kill child deadline overflowed")?;
    loop {
        if let Some(status) =
            try_wait(child).with_context(|| format!("polling terminated {label}"))?
        {
            return Ok(status);
        }
        let now = Instant::now();
        if now >= deadline {
            bail!(
                "{label} did not exit within {} ms after termination",
                timeout.as_millis()
            );
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
}

struct Capture {
    directory: PathBuf,
    stdout: CapturedFile,
    stderr: CapturedFile,
}

struct CapturedFile {
    path: PathBuf,
    file: File,
}

struct CaptureReservation {
    directory: PathBuf,
    paths: Vec<PathBuf>,
    committed: bool,
}

impl CaptureReservation {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            paths: Vec::new(),
            committed: false,
        }
    }

    fn track(&mut self, path: &std::path::Path) {
        self.paths.push(path.to_path_buf());
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for CaptureReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for path in self.paths.iter().rev() {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir(&self.directory);
    }
}

impl Capture {
    fn create() -> Result<(Self, File, File)> {
        Self::create_in(&std::env::temp_dir(), |_, _| Ok(()))
    }

    fn create_in(
        parent: &std::path::Path,
        mut before_stream: impl FnMut(&std::path::Path, &str) -> Result<()>,
    ) -> Result<(Self, File, File)> {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..128 {
            let sequence = NEXT_CAPTURE.fetch_add(1, Ordering::Relaxed);
            let directory = parent.join(format!(
                "wizard-xtask-capture-{}-{epoch}-{sequence}",
                std::process::id()
            ));
            match std::fs::create_dir(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("creating capture directory {}", directory.display())
                    });
                }
            }
            let stdout_path = directory.join("stdout");
            let stderr_path = directory.join("stderr");
            let mut reservation = CaptureReservation::new(directory.clone());
            before_stream(&stdout_path, "stdout")?;
            let (stdout_file, stdout) =
                Self::create_file(&mut reservation, &stdout_path, "stdout")?;
            before_stream(&stderr_path, "stderr")?;
            let (stderr_file, stderr) =
                Self::create_file(&mut reservation, &stderr_path, "stderr")?;
            let capture = Self {
                stdout: CapturedFile {
                    path: stdout_path,
                    file: stdout_file,
                },
                stderr: CapturedFile {
                    path: stderr_path,
                    file: stderr_file,
                },
                directory,
            };
            reservation.commit();
            return Ok((capture, stdout, stderr));
        }
        bail!("could not reserve a unique bounded-child capture directory")
    }

    fn create_file(
        reservation: &mut CaptureReservation,
        path: &std::path::Path,
        label: &str,
    ) -> Result<(File, File)> {
        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::read(&mut options, true);
        OpenOptions::write(&mut options, true);
        let file = OpenOptions::open(&options, path)
            .with_context(|| format!("creating {label} capture {}", path.display()))?;
        reservation.track(path);
        let writer = file
            .try_clone()
            .with_context(|| format!("retaining {label} capture {}", path.display()))?;
        Ok((file, writer))
    }

    fn collect_with_limit(&self, status: ExitStatus, limit: usize) -> Result<Output> {
        self.require_within_limit(limit)?;
        let stdout = read_limited(&self.stdout.file, &self.stdout.path, limit, "output")?;
        let stderr = read_limited(
            &self.stderr.file,
            &self.stderr.path,
            limit - stdout.len(),
            "diagnostic output",
        )?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    fn require_within_limit(&self, limit: usize) -> Result<()> {
        let stdout = self
            .stdout
            .file
            .metadata()
            .with_context(|| format!("inspecting captured output {}", self.stdout.path.display()))?
            .len();
        let stderr = self
            .stderr
            .file
            .metadata()
            .with_context(|| {
                format!(
                    "inspecting captured diagnostics {}",
                    self.stderr.path.display()
                )
            })?
            .len();
        let bytes = stdout
            .checked_add(stderr)
            .context("captured child output length overflowed")?;
        ensure!(
            bytes <= limit as u64,
            "bounded child output exceeded {limit} bytes"
        );
        Ok(())
    }
}

fn read_limited(
    retained: &File,
    path: &std::path::Path,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>> {
    let mut file = retained
        .try_clone()
        .with_context(|| format!("cloning captured {label} {}", path.display()))?;
    std::io::Seek::seek(&mut file, SeekFrom::Start(0))
        .with_context(|| format!("rewinding captured {label} {}", path.display()))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((limit as u64) + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading captured {label} {}", path.display()))?;
    ensure!(
        bytes.len() <= limit,
        "bounded child output exceeded its {limit}-byte remainder"
    );
    Ok(bytes)
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.stdout.path);
        let _ = std::fs::remove_file(&self.stderr.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{self, Write};
    use std::rc::Rc;

    use super::*;

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

    #[cfg(unix)]
    fn success_status() -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;

        ExitStatus::from_raw(0)
    }

    #[cfg(windows)]
    fn success_status() -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;

        ExitStatus::from_raw(0)
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
        let (capture, stdout, stderr) = Capture::create().unwrap();
        drop((stdout, stderr));
        std::fs::write(&capture.stdout.path, b"four").unwrap();
        std::fs::write(&capture.stderr.path, b"two").unwrap();

        let error = collect_output(&capture, success_status(), 5).unwrap_err();
        assert!(format!("{error:#}").contains("exceeded 5 bytes"));
    }

    #[test]
    fn a_second_stream_failure_rolls_back_the_first_capture_and_directory() {
        let parent = std::env::temp_dir().join(format!(
            "wizard-capture-rollback-{}-{}",
            std::process::id(),
            NEXT_CAPTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&parent).unwrap();
        let mut stream = 0;

        let error = match Capture::create_in(&parent, |_, _| {
            stream += 1;
            ensure!(stream != 2, "injected second-stream failure");
            Ok(())
        }) {
            Ok(_) => panic!("injected second-stream failure was accepted"),
            Err(error) => error,
        };

        assert!(format!("{error:#}").contains("injected second-stream failure"));
        assert!(crate::paths::directory_entries(&parent).unwrap().is_empty());
        std::fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn capture_growth_is_refused_while_the_child_is_still_polled() {
        let (capture, stdout, stderr) = Capture::create().unwrap();
        drop((stdout, stderr));
        std::fs::write(&capture.stdout.path, b"four").unwrap();
        std::fs::write(&capture.stderr.path, b"two").unwrap();

        let error = capture.require_within_limit(5).unwrap_err();
        assert!(format!("{error:#}").contains("exceeded 5 bytes"));
    }

    #[test]
    fn collection_reads_the_original_capture_handle_not_a_substituted_path() {
        let (capture, mut stdout, stderr) = Capture::create().unwrap();
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

    /// Starts the parked grandchild without waiting for it, sees its lock
    /// taken, then exits or lingers as the test asks.
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

    fn parked_family(label: &str, lingers: bool) -> (PathBuf, PathBuf, Command) {
        let root = std::env::temp_dir().join(format!(
            "wizard-parked-{label}-{}-{}",
            std::process::id(),
            NEXT_CAPTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
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
    fn git_children_cannot_inherit_repository_or_pathspec_redirection() {
        let mut command = Command::new("git");
        for name in GIT_REDIRECT_ENVIRONMENT {
            command.env(name, "attacker-controlled");
        }
        isolate_git_environment(&mut command);
        let environment: std::collections::BTreeMap<_, _> = command.get_envs().collect();

        for name in GIT_REDIRECT_ENVIRONMENT {
            assert_eq!(
                environment.get(std::ffi::OsStr::new(name)).copied(),
                Some(None),
                "{name} was still inherited"
            );
        }
        assert_eq!(
            environment
                .get(std::ffi::OsStr::new("GIT_CONFIG_COUNT"))
                .copied()
                .flatten(),
            Some(std::ffi::OsStr::new("0"))
        );
    }
}
