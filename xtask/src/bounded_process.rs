//! Bounded child execution without pipe-EOF waits.
//!
//! Regular-file capture prevents inherited pipe handles from extending a wait.
//! Termination gets its own bounded poll. On Windows every command runs in a
//! job object that holds each process it starts, and the whole job is
//! terminated when the command ends, times out or fails (D-347). The byte cap
//! bounds memory rather than disk retained through an unlinked capture file.
//! Line-framed dialogues also poll caller-owned live state under the same
//! deadline, so an external scratch-budget failure ends the job (D-406).

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, SeekFrom, Write};
use std::path::PathBuf;
#[cfg(not(windows))]
use std::process::Child;
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};

const POLL_INTERVAL: Duration = Duration::from_millis(25);
const POST_KILL_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
#[cfg(windows)]
const EMPTY_GIT_CONFIG: &str = "NUL";
#[cfg(not(windows))]
const EMPTY_GIT_CONFIG: &str = "/dev/null";
const FIXED_GIT_ENVIRONMENT: &[(&str, &str)] = &[
    ("GIT_ATTR_NOSYSTEM", "1"),
    ("GIT_CONFIG_COUNT", "0"),
    ("GIT_CONFIG_GLOBAL", EMPTY_GIT_CONFIG),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_SYSTEM", EMPTY_GIT_CONFIG),
    ("GIT_LITERAL_PATHSPECS", "1"),
    ("GIT_NO_LAZY_FETCH", "1"),
    ("GIT_NO_REPLACE_OBJECTS", "1"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_PAGER", ""),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("LANG", "C"),
    ("LC_ALL", "C"),
];
static NEXT_CAPTURE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn isolate_git_environment(command: &mut Command) {
    command
        .env_clear()
        .envs(FIXED_GIT_ENVIRONMENT.iter().copied());
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct DialogueLimits {
    pub(crate) line_bytes: usize,
    pub(crate) stdout_bytes: usize,
    pub(crate) stderr_bytes: usize,
}

pub(crate) struct LineDialogue {
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    limits: DialogueLimits,
    stdout_bytes: usize,
    label: String,
}

impl LineDialogue {
    pub(crate) fn send_line(&mut self, bytes: &[u8]) -> Result<()> {
        let input = self
            .input
            .as_mut()
            .with_context(|| format!("{} input is closed", self.label))?;
        write_bounded_line(input, bytes, self.limits.line_bytes, &self.label)
    }

    pub(crate) fn close_input(&mut self) {
        drop(self.input.take());
    }

    pub(crate) fn receive_line(&mut self) -> Result<Option<Vec<u8>>> {
        read_bounded_line(
            &mut self.output,
            self.limits.line_bytes,
            self.limits.stdout_bytes,
            &mut self.stdout_bytes,
            &self.label,
        )
    }
}

fn dialogue_deadline(start: Instant, timeout: Duration) -> Result<Instant> {
    start
        .checked_add(timeout)
        .context("bounded child deadline overflowed")
}

/// Run one line-framed dialogue through one deadline and capture diagnostics.
///
/// The dialogue runs off the polling thread so a blocked pipe cannot prevent
/// the complete child tree from being stopped at the original deadline.
/// The owner observer runs on that polling thread under the same deadline.
/// Failure cleanup may consume the additional bounded post-kill interval.
pub(crate) fn output_dialogue<T, O, F>(
    mut command: Command,
    timeout: Duration,
    label: &str,
    limits: DialogueLimits,
    mut observe_owner: O,
    interact: F,
) -> Result<StreamedOutput<T>>
where
    T: Send + 'static,
    O: FnMut() -> Result<()>,
    F: FnOnce(&mut LineDialogue) -> Result<T> + Send + 'static,
{
    let deadline = dialogue_deadline(Instant::now(), timeout)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut tree = Tree::spawn(command).with_context(|| format!("launching {label}"))?;
    let Some(input) = tree.take_stdin() else {
        return stop_tree_after_setup_failure(
            tree,
            anyhow::anyhow!("{label} has no piped stdin"),
            label,
        );
    };
    let Some(output) = tree.take_stdout() else {
        return stop_tree_after_setup_failure(
            tree,
            anyhow::anyhow!("{label} has no piped stdout"),
            label,
        );
    };
    let Some(stderr) = tree.take_stderr() else {
        return stop_tree_after_setup_failure(
            tree,
            anyhow::anyhow!("{label} has no piped stderr"),
            label,
        );
    };

    let (dialogue_sender, dialogue_receiver) = mpsc::sync_channel(1);
    let dialogue_label = label.to_string();
    let dialogue_thread = match std::thread::Builder::new()
        .name("wizard-bounded-dialogue".to_string())
        .spawn(move || {
            let mut dialogue = LineDialogue {
                input: Some(input),
                output: BufReader::new(output),
                limits,
                stdout_bytes: 0,
                label: dialogue_label,
            };
            let result = interact(&mut dialogue);
            drop(dialogue);
            let _ = dialogue_sender.send(result);
        }) {
        Ok(thread) => thread,
        Err(error) => {
            return stop_tree_after_setup_failure(
                tree,
                anyhow::Error::new(error).context(format!("starting {label} dialogue")),
                label,
            );
        }
    };
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    let stderr_label = label.to_string();
    let stderr_thread = match std::thread::Builder::new()
        .name("wizard-bounded-stderr".to_string())
        .spawn(move || {
            let result = read_pipe_with_limit(stderr, limits.stderr_bytes, &stderr_label);
            let _ = stderr_sender.send(result);
        }) {
        Ok(thread) => thread,
        Err(error) => {
            return stop_tree_after_setup_failure(
                tree,
                anyhow::Error::new(error).context(format!("starting {label} stderr reader")),
                label,
            );
        }
    };

    let mut dialogue_value = None;
    let mut stderr_value = None;
    let status = poll_child_until(
        tree,
        Tree::try_wait,
        Tree::terminate,
        deadline,
        timeout,
        label,
        || {
            observe_owner().with_context(|| format!("observing {label} owner state"))?;
            receive_ready(
                &dialogue_receiver,
                &mut dialogue_value,
                &format!("{label} dialogue"),
            )?;
            receive_ready(
                &stderr_receiver,
                &mut stderr_value,
                &format!("{label} stderr reader"),
            )
        },
    )?;

    let value = receive_after_exit(
        &dialogue_receiver,
        dialogue_value,
        &format!("{label} dialogue"),
    )?;
    let stderr = receive_after_exit(
        &stderr_receiver,
        stderr_value,
        &format!("{label} stderr reader"),
    )?;
    join_reader(dialogue_thread, &format!("{label} dialogue"))?;
    join_reader(stderr_thread, &format!("{label} stderr reader"))?;
    Ok(StreamedOutput {
        status,
        value,
        stderr,
    })
}

fn write_bounded_line(
    writer: &mut impl Write,
    bytes: &[u8],
    line_limit: usize,
    label: &str,
) -> Result<()> {
    ensure!(
        !bytes.contains(&b'\n') && !bytes.contains(&b'\r'),
        "{label} request contains a raw line ending"
    );
    let framed = bytes
        .len()
        .checked_add(1)
        .context("bounded dialogue request length overflowed")?;
    ensure!(
        framed <= line_limit,
        "{label} request exceeds the {line_limit}-byte line limit"
    );
    writer
        .write_all(bytes)
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
        .with_context(|| format!("writing {label} request"))
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    line_limit: usize,
    total_limit: usize,
    total: &mut usize,
    label: &str,
) -> Result<Option<Vec<u8>>> {
    let maximum = u64::try_from(line_limit)
        .context("bounded dialogue line limit does not fit u64")?
        .checked_add(1)
        .context("bounded dialogue line limit overflowed")?;
    let mut line = Vec::new();
    let read = reader
        .take(maximum)
        .read_until(b'\n', &mut line)
        .with_context(|| format!("reading {label} response"))?;
    if read == 0 {
        return Ok(None);
    }
    ensure!(
        line.len() <= line_limit,
        "{label} sent a line longer than {line_limit} bytes"
    );
    let observed = total
        .checked_add(line.len())
        .context("bounded dialogue stdout length overflowed")?;
    ensure!(
        observed <= total_limit,
        "{label} stdout exceeded {total_limit} bytes"
    );
    *total = observed;
    Ok(Some(line))
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

    fn take_stdin(&mut self) -> Option<ChildStdin> {
        #[cfg(windows)]
        {
            self.child.inner().stdin.take()
        }
        #[cfg(not(windows))]
        {
            self.child.stdin.take()
        }
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
    child: C,
    try_wait: fn(&mut C) -> std::io::Result<Option<ExitStatus>>,
    kill: fn(&mut C) -> std::io::Result<()>,
    timeout: Duration,
    label: &str,
    observe: F,
) -> Result<ExitStatus>
where
    F: FnMut() -> Result<()>,
{
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("bounded child deadline overflowed")?;
    poll_child_until(child, try_wait, kill, deadline, timeout, label, observe)
}

fn poll_child_until<C, F>(
    mut child: C,
    try_wait: fn(&mut C) -> std::io::Result<Option<ExitStatus>>,
    kill: fn(&mut C) -> std::io::Result<()>,
    deadline: Instant,
    timeout: Duration,
    label: &str,
    mut observe: F,
) -> Result<ExitStatus>
where
    F: FnMut() -> Result<()>,
{
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

fn stop_tree_after_setup_failure<T>(tree: Tree, error: anyhow::Error, label: &str) -> Result<T> {
    match stop_after_failure(
        tree,
        Tree::try_wait,
        Tree::terminate,
        error,
        label,
        "setup-failed",
    ) {
        Err(error) => Err(error),
        Ok(status) => bail!("setup-failed {label} unexpectedly returned {status}"),
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
    Read::by_ref(&mut file)
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
    use std::ffi::OsString;
    use std::io::{self, Write};
    use std::rc::Rc;

    use super::*;

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
            let error = read_bounded_line(&mut reader, usize::MAX, usize::MAX, &mut 0, "fixture")
                .unwrap_err();
            assert!(format!("{error:#}").contains("line limit overflowed"));
        }

        let mut reader = BufReader::new(io::Cursor::new(b"x"));
        let mut total = usize::MAX;
        let error =
            read_bounded_line(&mut reader, 1, usize::MAX, &mut total, "fixture").unwrap_err();
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
        let (capture, stdout, stderr) = Capture::create().unwrap();
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
}
