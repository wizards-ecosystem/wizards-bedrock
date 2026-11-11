//! Bounded child execution without pipe-EOF waits.
//! Regular-file capture keeps inherited pipe handles from extending a wait.
//! On Windows one job object holds every process a command starts, and the whole job is
//! terminated when the command ends, times out or fails.
//! The byte cap bounds memory, not disk retained through an unlinked capture file.
//! Dialogues poll caller-owned live state under the same deadline, so an external
//! scratch-budget failure ends the job.

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

/// Failure cleanup may take the bounded post-kill interval beyond the deadline.
pub(crate) fn output(command: Command, timeout: Duration, label: &str) -> Result<Output> {
    output_with_limit(command, timeout, label, MAX_CAPTURE_BYTES)
}

/// The capture limit covers stdout and stderr combined.
pub(crate) fn output_with_limit(
    mut command: Command,
    timeout: Duration,
    label: &str,
    capture_limit: usize,
) -> Result<Output> {
    let (capture, stdout, stderr) = Capture::reserve()?;
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

#[derive(Debug)]
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

/// The dialogue runs off the polling thread, so a blocked pipe cannot keep the whole child tree
/// from being stopped at the original deadline.
/// The owner observer runs on the polling thread under the same deadline.
/// Failure cleanup may take the bounded post-kill interval beyond the deadline.
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

    let dialogue_label = label.to_string();
    let mut dialogue = match Reader::spawn(
        "wizard-bounded-dialogue",
        format!("{label} dialogue"),
        move || {
            let mut dialogue = LineDialogue {
                input: Some(input),
                output: BufReader::new(output),
                limits,
                stdout_bytes: 0,
                label: dialogue_label,
            };
            let value = interact(&mut dialogue);
            drop(dialogue);
            value
        },
    ) {
        Ok(reader) => reader,
        Err(error) => return stop_tree_after_setup_failure(tree, error, label),
    };
    let mut stderr = match stderr_reader(stderr, limits.stderr_bytes, label) {
        Ok(reader) => reader,
        Err(error) => return stop_tree_after_setup_failure(tree, error, label),
    };

    let status = poll_child_until(
        tree,
        Tree::try_wait,
        Tree::terminate,
        deadline,
        timeout,
        label,
        || {
            observe_owner().with_context(|| format!("observing {label} owner state"))?;
            dialogue.poll()?;
            stderr.poll()
        },
    )?;
    collect_readers(status, dialogue, stderr)
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

/// The consumer must bound each record before allocating it.
/// Reader errors are observed while the child is live, so the whole tree is stopped first.
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

    let mut reducer = Reader::spawn(
        "wizard-bounded-stdout",
        format!("{label} stdout reducer"),
        move || consume_stdout(stdout),
    )?;
    let mut stderr = stderr_reader(stderr, stderr_limit, label)?;

    let status = poll_child_with(
        tree,
        Tree::try_wait,
        Tree::terminate,
        timeout,
        label,
        || {
            reducer.poll()?;
            stderr.poll()
        },
    )?;
    collect_readers(status, reducer, stderr)
}

#[derive(Debug)]
struct Reader<T> {
    label: String,
    thread: JoinHandle<()>,
    receiver: Receiver<Result<T>>,
    value: Option<T>,
}

impl<T: Send + 'static> Reader<T> {
    fn spawn(
        thread_name: &str,
        label: String,
        work: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(move || {
                let _ = sender.send(work());
            })
            .with_context(|| format!("starting {label}"))?;
        Ok(Self {
            label,
            thread,
            receiver,
            value: None,
        })
    }

    fn poll(&mut self) -> Result<()> {
        receive_ready(&self.receiver, &mut self.value, &self.label)
    }
}

fn stderr_reader(stderr: ChildStderr, limit: usize, label: &str) -> Result<Reader<Vec<u8>>> {
    let pipe_label = label.to_string();
    Reader::spawn(
        "wizard-bounded-stderr",
        format!("{label} stderr reader"),
        move || read_pipe_with_limit(stderr, limit, &pipe_label),
    )
}

fn collect_readers<T>(
    status: ExitStatus,
    reader: Reader<T>,
    stderr: Reader<Vec<u8>>,
) -> Result<StreamedOutput<T>> {
    let value = receive_after_exit(&reader.receiver, reader.value, &reader.label)?;
    let diagnostics = receive_after_exit(&stderr.receiver, stderr.value, &stderr.label)?;
    join_reader(reader.thread, &reader.label)?;
    join_reader(stderr.thread, &stderr.label)?;
    Ok(StreamedOutput {
        status,
        value,
        stderr: diagnostics,
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

/// On Windows one job holds every process the command starts, and dropping the tree terminates
/// the whole job, so no descendant outlives the command however it ends.
#[derive(Debug)]
struct Tree {
    #[cfg(windows)]
    child: command_group::GroupChild,
    #[cfg(not(windows))]
    child: Child,
}

impl Tree {
    /// On Windows the child is created suspended, assigned to its job and only then resumed,
    /// so nothing it starts can escape the job.
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

#[derive(Debug)]
struct Capture {
    directory: PathBuf,
    stdout: CapturedFile,
    stderr: CapturedFile,
}

#[derive(Debug)]
struct CapturedFile {
    path: PathBuf,
    file: File,
}

#[derive(Debug)]
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
    fn reserve() -> Result<(Self, File, File)> {
        Self::reserve_in(&std::env::temp_dir(), |_, _| Ok(()))
    }

    fn reserve_in(
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
                Self::reserve_file(&mut reservation, &stdout_path, "stdout")?;
            before_stream(&stderr_path, "stderr")?;
            let (stderr_file, stderr) =
                Self::reserve_file(&mut reservation, &stderr_path, "stderr")?;
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

    fn reserve_file(
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
mod tests;
