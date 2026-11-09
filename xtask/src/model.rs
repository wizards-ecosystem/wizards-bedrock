//! TLC qualification for the queue protocol models.
//!
//! `Ring.tla` checks anonymous admission; `RingMemory.tla` checks weak-memory
//! publication through normal, cancelled and retired operations (D-79, D-221,
//! D-229). The task remains outside `ci` because its JVM cannot be vendored;
//! phase-close qualification records the external execution instead.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};

use crate::paths::Paths;
use crate::qualification;
use crate::sha256;

/// The pinned TLA+ tools jar, relative to the workspace root.
pub const JAR: &str = "tools/tla/tla2tools.jar";

/// Its SHA-256, pinned the way `boot/limine`'s binaries are (D-25).
///
/// TLC is a tool that decides whether a design is correct, so a silently
/// swapped one is a silently swapped answer.
pub const JAR_DIGEST: &str = "eabd140a70f49eb9305a3bd3f3df944eddf87e5a90d329789085f8953a80533a";

/// The model directory, relative to the workspace root.
const MODEL_DIR: &str = "models";
const TRANSCRIPT_FORMAT: &str = "wizard-model-transcript-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptArtifact {
    pub(crate) prefix: &'static str,
    pub(crate) name: &'static str,
}

pub(crate) const TRANSCRIPT_ARTIFACTS: &[TranscriptArtifact] = &[
    TranscriptArtifact {
        prefix: "artifact.run.ring-clean",
        name: "ring-clean.transcript",
    },
    TranscriptArtifact {
        prefix: "artifact.run.ring-memory-clean",
        name: "ring-memory-clean.transcript",
    },
    TranscriptArtifact {
        prefix: "artifact.run.ring-memory-store-relaxed",
        name: "ring-memory-store-relaxed.transcript",
    },
    TranscriptArtifact {
        prefix: "artifact.run.ring-memory-load-relaxed",
        name: "ring-memory-load-relaxed.transcript",
    },
    TranscriptArtifact {
        prefix: "artifact.run.ring-memory-no-reservation",
        name: "ring-memory-no-reservation.transcript",
    },
    TranscriptArtifact {
        prefix: "artifact.run.ring-memory-duplicate-terminal",
        name: "ring-memory-duplicate-terminal.transcript",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expected {
    Clean,
    Violates(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    artifact: TranscriptArtifact,
    model: &'static str,
    config: &'static str,
    expected: Expected,
}

const RUNS: &[Run] = &[
    Run {
        artifact: TRANSCRIPT_ARTIFACTS[0],
        model: "Ring.tla",
        config: "Ring.cfg",
        expected: Expected::Clean,
    },
    Run {
        artifact: TRANSCRIPT_ARTIFACTS[1],
        model: "RingMemory.tla",
        config: "RingMemory.cfg",
        expected: Expected::Clean,
    },
    Run {
        artifact: TRANSCRIPT_ARTIFACTS[2],
        model: "RingMemory.tla",
        config: "RingMemory.store-relaxed.cfg",
        expected: Expected::Violates("NoStalePayload"),
    },
    Run {
        artifact: TRANSCRIPT_ARTIFACTS[3],
        model: "RingMemory.tla",
        config: "RingMemory.load-relaxed.cfg",
        expected: Expected::Violates("NoStalePayload"),
    },
    Run {
        artifact: TRANSCRIPT_ARTIFACTS[4],
        model: "RingMemory.tla",
        config: "RingMemory.no-reservation.cfg",
        expected: Expected::Violates("ReservationsAreLive"),
    },
    Run {
        artifact: TRANSCRIPT_ARTIFACTS[5],
        model: "RingMemory.tla",
        config: "RingMemory.duplicate-terminal.cfg",
        expected: Expected::Violates("OneCompletionPerOperation"),
    },
];

/// What TLC prints when every invariant held in every reachable state.
const CLEAN: &str = "Model checking completed. No error has been found.";
const BEHAVIOR: &str = "Error: The behavior up to this point is:";
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Clean,
    Violated(String),
}

#[derive(Debug, Clone)]
pub(crate) struct RunTranscript {
    artifact: TranscriptArtifact,
    bytes: Vec<u8>,
}

impl RunTranscript {
    pub(crate) fn artifact(&self) -> TranscriptArtifact {
        self.artifact
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Check both models and require every deliberate mutation to fail exactly.
///
/// # Errors
///
/// If the jar is missing or its digest has changed, if `java` cannot be run,
/// or if TLC reports anything other than the expected clean or exact-mutant
/// verdict.
pub fn check(paths: &Paths) -> Result<()> {
    let qualification_attempt = qualification::begin_model(paths)?;
    let jar = paths.root().join(JAR);
    verify_jar(&jar)?;
    let java = qualification_attempt.tool();

    let models = paths.root().join(MODEL_DIR);
    println!(
        "[xtask] Java identity: {}",
        java.version.replace('\n', " | ")
    );
    let mut transcripts = Vec::with_capacity(RUNS.len());
    for run in RUNS {
        println!(
            "[xtask] model-check: TLC over {MODEL_DIR}/{} with {}",
            run.model, run.config
        );
        let scratch = prepare_scratch(paths, run)?;
        let mut command = Command::new(&java.path);
        command
            .current_dir(&models)
            // TLC's default collector spends most of a short run in GC.
            .arg("-XX:+UseParallelGC")
            .arg("-cp")
            .arg(&jar)
            .arg("tlc2.TLC")
            .arg("-cleanup")
            .arg("-noGenerateSpecTE")
            .arg("-metadir")
            .arg(&scratch)
            .arg("-config")
            .arg(run.config)
            .arg(run.model);
        let output = bounded_output(command, RUN_TIMEOUT, run.config).with_context(|| {
            format!("could not run the resolved JVM at {}", java.path.display())
        })?;
        let success = output.status.success();
        let report = String::from_utf8(output.stdout).context("TLC output is not UTF-8")?;
        let error = String::from_utf8(output.stderr).context("TLC diagnostics are not UTF-8")?;
        if let Err(failure) = require_expected(run.expected, success, &report, &error) {
            print!("{report}");
            if !error.is_empty() {
                eprint!("{error}");
            }
            return Err(failure).with_context(|| {
                format!(
                    "TLC did not classify {} with {} exactly",
                    run.model, run.config
                )
            });
        }
        transcripts.push(transcript(
            run,
            success,
            report.as_bytes(),
            error.as_bytes(),
        )?);
        match run.expected {
            Expected::Clean => print!("{report}"),
            Expected::Violates(invariant) => println!(
                "[xtask] model mutation {} violated {invariant} exactly",
                run.config
            ),
        }
    }
    qualification_attempt.pass(&transcripts)?;
    println!("[xtask] model-check passed with every deliberate mutation caught");
    Ok(())
}

fn expected_name(expected: Expected) -> String {
    match expected {
        Expected::Clean => "clean".to_string(),
        Expected::Violates(invariant) => format!("violates:{invariant}"),
    }
}

fn transcript(run: &Run, success: bool, stdout: &[u8], stderr: &[u8]) -> Result<RunTranscript> {
    transcript_with_capture_limit(
        run,
        success,
        stdout,
        stderr,
        crate::bounded_process::MAX_CAPTURE_BYTES,
    )
}

fn transcript_with_capture_limit(
    run: &Run,
    success: bool,
    stdout: &[u8],
    stderr: &[u8],
    maximum: usize,
) -> Result<RunTranscript> {
    let captured = stdout
        .len()
        .checked_add(stderr.len())
        .context("TLC transcript length overflowed")?;
    ensure!(
        captured <= maximum,
        "TLC transcript exceeded the bounded child capture limit"
    );
    let bytes = render_transcript(run, success, stdout, stderr);
    verify_transcript(run.artifact.name, &bytes)?;
    Ok(RunTranscript {
        artifact: run.artifact,
        bytes,
    })
}

fn render_transcript(run: &Run, success: bool, stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut rendered = format!(
        "{TRANSCRIPT_FORMAT}\nconfig={}\nexpected={}\nexit.success={}\nmodel={}\nstderr.bytes={}\nstderr.sha256={}\nstdout.bytes={}\nstdout.sha256={}\n\n",
        run.config,
        expected_name(run.expected),
        success,
        run.model,
        stderr.len(),
        sha256::hex(&sha256::digest(stderr)),
        stdout.len(),
        sha256::hex(&sha256::digest(stdout)),
    )
    .into_bytes();
    rendered.extend_from_slice(stdout);
    rendered.extend_from_slice(stderr);
    rendered
}

pub(crate) fn require_complete_transcripts(transcripts: &[RunTranscript]) -> Result<()> {
    ensure!(
        transcripts.len() == RUNS.len(),
        "model qualification has {} transcripts, expected {}",
        transcripts.len(),
        RUNS.len()
    );
    for (run, transcript) in RUNS.iter().zip(transcripts) {
        ensure!(
            transcript.artifact == run.artifact,
            "model qualification transcripts are missing or reordered"
        );
        verify_transcript(run.artifact.name, &transcript.bytes)?;
    }
    Ok(())
}

pub(crate) fn verify_transcript(name: &str, bytes: &[u8]) -> Result<()> {
    verify_transcript_with_capture_limit(name, bytes, crate::bounded_process::MAX_CAPTURE_BYTES)
}

fn verify_transcript_with_capture_limit(name: &str, bytes: &[u8], maximum: usize) -> Result<()> {
    let run = RUNS
        .iter()
        .find(|run| run.artifact.name == name)
        .with_context(|| format!("unregistered model transcript `{name}`"))?;
    let separator = bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .context("model transcript has no header terminator")?;
    let header =
        std::str::from_utf8(&bytes[..separator]).context("model transcript header is not UTF-8")?;
    ensure!(
        !header.contains(['\r', '\0']),
        "model transcript header is not canonical LF text"
    );
    let lines: Vec<&str> = header.lines().collect();
    ensure!(
        lines.len() == 9 && lines[0] == TRANSCRIPT_FORMAT,
        "model transcript has no exact supported header"
    );
    let field = |index: usize, prefix: &str| -> Result<&str> {
        lines[index]
            .strip_prefix(prefix)
            .with_context(|| format!("model transcript field {index} is not `{prefix}`"))
    };
    let config = field(1, "config=")?;
    let expected = field(2, "expected=")?;
    let success = field(3, "exit.success=")?;
    let model = field(4, "model=")?;
    let stderr_bytes = parse_canonical_count(field(5, "stderr.bytes=")?)?;
    let stderr_digest = field(6, "stderr.sha256=")?;
    let stdout_bytes = parse_canonical_count(field(7, "stdout.bytes=")?)?;
    let stdout_digest = field(8, "stdout.sha256=")?;
    ensure!(
        config == run.config
            && model == run.model
            && expected == expected_name(run.expected)
            && matches!(success, "true" | "false"),
        "model transcript identity does not match its registered run"
    );
    let success = success == "true";
    let payload = &bytes[separator + 2..];
    let captured = stdout_bytes
        .checked_add(stderr_bytes)
        .context("model transcript payload length overflowed")?;
    ensure!(
        captured == payload.len(),
        "model transcript payload length is not exact"
    );
    ensure!(
        captured <= maximum,
        "model transcript payload exceeds the bounded child capture limit"
    );
    let (stdout, stderr) = payload.split_at(stdout_bytes);
    ensure!(
        sha256::hex(&sha256::digest(stdout)) == stdout_digest
            && sha256::hex(&sha256::digest(stderr)) == stderr_digest,
        "model transcript stream checksum does not match"
    );
    let stdout = std::str::from_utf8(stdout).context("model transcript stdout is not UTF-8")?;
    let stderr = std::str::from_utf8(stderr).context("model transcript stderr is not UTF-8")?;
    require_expected(run.expected, success, stdout, stderr)?;
    ensure!(
        render_transcript(run, success, stdout.as_bytes(), stderr.as_bytes()) == bytes,
        "model transcript is not in its canonical encoding"
    );
    Ok(())
}

fn parse_canonical_count(value: &str) -> Result<usize> {
    let count = <usize as core::str::FromStr>::from_str(value)
        .context("model transcript byte count is not a number")?;
    ensure!(
        count.to_string() == value,
        "model transcript byte count is not canonical"
    );
    Ok(count)
}

#[cfg(test)]
pub(crate) fn fixture_transcripts() -> Vec<RunTranscript> {
    RUNS.iter()
        .map(|run| {
            let (success, stdout) = match run.expected {
                Expected::Clean => (
                    true,
                    format!(
                        "Starting...\n{CLEAN}\n1 states generated.\nFinished in 00s at (2026-08-30 00:00:00)\n"
                    ),
                ),
                Expected::Violates(invariant) => (
                    false,
                    format!(
                        "Starting...\nError: Invariant {invariant} is violated.\n{BEHAVIOR}\nState 1: <Initial predicate>\nFinished in 00s at (2026-08-30 00:00:00)\n"
                    ),
                ),
            };
            transcript(run, success, stdout.as_bytes(), b"").expect("fixture transcript")
        })
        .collect()
}

fn prepare_scratch(paths: &Paths, run: &Run) -> Result<PathBuf> {
    ensure!(RUNS.contains(run), "refusing an unregistered model run");
    let mut components = Path::new(run.config).components();
    ensure!(
        matches!(
            core::iter::Iterator::next(&mut components),
            Some(Component::Normal(_))
        ) && core::iter::Iterator::next(&mut components).is_none(),
        "model configuration is not one safe scratch selector"
    );
    let target = crate::mutate::workspace::prepare_real_directory_tree(
        paths.target_dir(),
        "model-check target directory",
    )?;
    let root = crate::mutate::workspace::prepare_real_child(
        &target,
        &target.join("model-check"),
        "model-check state directory",
    )?;
    let scratch = root.join(run.config);
    match std::fs::symlink_metadata(&scratch) {
        Ok(_) => {
            let canonical = canonical_model_scratch(&scratch)?;
            std::fs::remove_dir_all(&canonical)
                .with_context(|| format!("resetting model scratch {}", canonical.display()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", scratch.display()));
        }
    }
    crate::mutate::workspace::prepare_real_child(&root, &scratch, "model-check scratch directory")
}

fn canonical_model_scratch(scratch: &Path) -> Result<PathBuf> {
    crate::mutate::workspace::canonical_existing_real_directory(
        scratch,
        "model-check scratch directory",
    )
}

fn bounded_output(command: Command, timeout: Duration, label: &str) -> Result<Output> {
    crate::bounded_process::output(command, timeout, label)
}

fn require_expected(expected: Expected, success: bool, stdout: &str, stderr: &str) -> Result<()> {
    let observed = classify(stdout, stderr)?;
    match (expected, observed) {
        (Expected::Clean, Verdict::Clean) if success => Ok(()),
        (Expected::Violates(expected), Verdict::Violated(actual))
            if !success && actual == expected =>
        {
            Ok(())
        }
        (Expected::Clean, Verdict::Clean) => bail!("a clean TLC verdict exited unsuccessfully"),
        (Expected::Violates(_), Verdict::Violated(_)) if success => {
            bail!("an invariant violation exited successfully")
        }
        (Expected::Clean, Verdict::Violated(actual)) => {
            bail!("the clean model violated `{actual}`")
        }
        (Expected::Violates(expected), Verdict::Violated(actual)) => {
            bail!("expected `{expected}` but TLC violated `{actual}`")
        }
        (Expected::Violates(expected), Verdict::Clean) => {
            bail!("mutation expected `{expected}` but TLC reported clean")
        }
    }
}

fn classify(stdout: &str, stderr: &str) -> Result<Verdict> {
    ensure!(
        stderr.trim().is_empty(),
        "TLC wrote diagnostic output beside its verdict"
    );
    let lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    ensure!(!lines.is_empty(), "TLC printed no output");
    ensure!(
        !lines.iter().any(|line| is_runtime_failure(line)),
        "TLC reported a runtime failure"
    );
    ensure!(
        !lines.iter().any(|line| {
            line.starts_with("Error:")
                && line != &BEHAVIOR
                && !(line.starts_with("Error: Invariant ") && line.ends_with(" is violated."))
        }),
        "TLC reported an unrelated error beside its verdict"
    );

    let finished: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (line.starts_with("Finished in ") && line.contains(" at (") && line.ends_with(')'))
                .then_some(index)
        })
        .collect();
    ensure!(
        finished.len() == 1 && finished[0] + 1 == lines.len(),
        "TLC has no single final completion record"
    );

    let mut verdict = None;
    for (index, line) in lines.iter().enumerate() {
        let candidate = if *line == CLEAN {
            Some(Verdict::Clean)
        } else {
            line.strip_prefix("Error: Invariant ")
                .and_then(|name| name.strip_suffix(" is violated."))
                .map(|name| Verdict::Violated(name.to_string()))
        };
        if let Some(candidate) = candidate {
            ensure!(index < finished[0], "TLC verdict follows its completion");
            ensure!(verdict.is_none(), "TLC printed mixed verdicts");
            verdict = Some((index, candidate));
        }
    }
    let (verdict_index, verdict) = verdict.context("TLC printed no terminal verdict")?;
    let behavior: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (*line == BEHAVIOR).then_some(index))
        .collect();
    let mut states = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with("State ") {
            states.push((index, trace_state_number(line)?));
        }
    }
    let trace_is_exact = match &verdict {
        Verdict::Clean => behavior.is_empty() && states.is_empty(),
        Verdict::Violated(_) => {
            behavior.len() == 1
                && verdict_index < behavior[0]
                && behavior[0] < finished[0]
                && states.first().map(|(index, _)| *index) == Some(behavior[0] + 1)
                && states.iter().enumerate().all(|(offset, (index, number))| {
                    behavior[0] < *index && *index < finished[0] && *number == offset + 1
                })
        }
    };
    ensure!(
        trace_is_exact,
        "TLC terminal verdict has no one exact matching behavior trace"
    );
    if let Verdict::Violated(name) = &verdict {
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "TLC named a malformed invariant"
        );
    }
    Ok(verdict)
}

fn trace_state_number(line: &str) -> Result<usize> {
    let record = line
        .strip_prefix("State ")
        .context("TLC trace state has no supported prefix")?;
    let (number, description) = record
        .split_once(": ")
        .context("TLC trace state has no numeric record terminator")?;
    let parsed = <usize as core::str::FromStr>::from_str(number)
        .context("TLC trace state number is malformed")?;
    ensure!(
        parsed != 0 && parsed.to_string() == number,
        "TLC trace state number is not canonical"
    );
    ensure!(
        description.len() > 2 && description.starts_with('<') && description.ends_with('>'),
        "TLC trace state has no canonical action description"
    );
    Ok(parsed)
}

fn is_runtime_failure(line: &str) -> bool {
    let throwable_name = line.split_once(':').map_or(line, |(name, _)| name).trim();
    let throwable = throwable_name != "Error"
        && !throwable_name.contains(char::is_whitespace)
        && ["Exception", "Error", "Throwable"]
            .iter()
            .any(|suffix| throwable_name.ends_with(suffix));
    let fatal = line
        .trim_start_matches('#')
        .trim_start()
        .to_ascii_lowercase();
    line.starts_with("Exception in thread")
        || line.starts_with("Caused by:")
        || line.starts_with("Suppressed:")
        || line.starts_with("at ")
        || throwable
        || line.contains("OutOfMemoryError")
        || line.contains("StackOverflowError")
        || line.starts_with("Error: TLC threw")
        || fatal.starts_with("fatal")
        || fatal.starts_with("a fatal error")
        || fatal.contains("fatal error")
}

/// Require the jar to be present and to be the one that was pinned.
fn verify_jar(jar: &Path) -> Result<()> {
    let file = std::fs::File::open(jar).with_context(|| {
        format!(
            "{} is missing; see tools/tla/README.md for where it comes from",
            jar.display()
        )
    })?;
    let (_, digest) = sha256::digest_reader(file)
        .with_context(|| format!("hashing pinned TLC jar {}", jar.display()))?;
    let digest = sha256::hex(&digest);
    if digest != JAR_DIGEST {
        bail!(
            "{} has digest {digest}, not the pinned {JAR_DIGEST}",
            jar.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
