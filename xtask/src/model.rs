//! TLC check of the queue protocol models; outside `ci` because its JVM cannot be vendored.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::paths::Paths;
use crate::sha256;

/// The pinned TLA+ tools jar, relative to the workspace root.
pub const JAR: &str = "tools/tla/tla2tools.jar";

/// Its SHA-256, pinned like `boot/limine`'s binaries: a silently swapped TLC is a swapped answer.
pub const JAR_DIGEST: &str = "eabd140a70f49eb9305a3bd3f3df944eddf87e5a90d329789085f8953a80533a";

const MODEL_DIR: &str = "models";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Expected {
    Clean,
    Violates(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Run {
    model: &'static str,
    config: &'static str,
    expected: Expected,
}

const RUNS: &[Run] = &[
    Run {
        model: "Ring.tla",
        config: "Ring.cfg",
        expected: Expected::Clean,
    },
    Run {
        model: "RingMemory.tla",
        config: "RingMemory.cfg",
        expected: Expected::Clean,
    },
    Run {
        model: "RingMemory.tla",
        config: "RingMemory.store-relaxed.cfg",
        expected: Expected::Violates("NoStalePayload"),
    },
    Run {
        model: "RingMemory.tla",
        config: "RingMemory.load-relaxed.cfg",
        expected: Expected::Violates("NoStalePayload"),
    },
    Run {
        model: "RingMemory.tla",
        config: "RingMemory.no-reservation.cfg",
        expected: Expected::Violates("ReservationsAreLive"),
    },
    Run {
        model: "RingMemory.tla",
        config: "RingMemory.duplicate-terminal.cfg",
        expected: Expected::Violates("OneCompletionPerOperation"),
    },
];

const CLEAN: &str = "Model checking completed. No error has been found.";
const BEHAVIOR: &str = "Error: The behavior up to this point is:";
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, PartialEq, Eq, Debug)]
enum Verdict {
    Clean,
    Violated(String),
}

/// Check both models and require every deliberate mutation to fail exactly.
pub fn check(paths: &Paths) -> Result<()> {
    let jar = paths.root().join(JAR);
    verify_jar(&jar)?;
    let java = java()?;

    let models = paths.root().join(MODEL_DIR);
    for run in RUNS {
        println!(
            "[xtask] model-check: TLC over {MODEL_DIR}/{} with {}",
            run.model, run.config
        );
        let scratch = prepare_scratch(paths, run)?;
        let mut command = Command::new(&java);
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
        let output = bounded_output(command, RUN_TIMEOUT, run.config)
            .with_context(|| format!("could not run the resolved JVM at {}", java.display()))?;
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
        match run.expected {
            Expected::Clean => print!("{report}"),
            Expected::Violates(invariant) => println!(
                "[xtask] model mutation {} violated {invariant} exactly",
                run.config
            ),
        }
    }
    println!("[xtask] model-check passed with every deliberate mutation caught");
    Ok(())
}

fn java() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let name = format!("java{}", std::env::consts::EXE_SUFFIX);
    crate::qemu::which_in(std::env::split_paths(&path), &name).context(
        "Java is unavailable; put the JVM that runs TLC (java) on PATH; \
         tools/tla/tla2tools.jar is TLC itself, not a JVM",
    )
}

fn prepare_scratch(paths: &Paths, run: &Run) -> Result<PathBuf> {
    ensure!(RUNS.contains(run), "refusing an unregistered model run");
    let mut components = Path::new(run.config).components();
    ensure!(
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none(),
        "model configuration is not one safe scratch selector"
    );
    let target = crate::workspace::prepare_real_directory_tree(
        paths.target_dir(),
        "model-check target directory",
    )?;
    let root = crate::workspace::prepare_real_child(
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
    crate::workspace::prepare_real_child(&root, &scratch, "model-check scratch directory")
}

fn canonical_model_scratch(scratch: &Path) -> Result<PathBuf> {
    crate::workspace::canonical_existing_real_directory(scratch, "model-check scratch directory")
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

    let finished = final_completion(&lines)?;
    let (verdict_index, verdict) = terminal_verdict(&lines, finished)?;
    let trace_is_exact = trace_matches(&lines, &verdict, verdict_index, finished)?;
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

fn final_completion(lines: &[&str]) -> Result<usize> {
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
    Ok(finished[0])
}

fn terminal_verdict(lines: &[&str], finished: usize) -> Result<(usize, Verdict)> {
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
            ensure!(index < finished, "TLC verdict follows its completion");
            ensure!(verdict.is_none(), "TLC printed mixed verdicts");
            verdict = Some((index, candidate));
        }
    }
    verdict.context("TLC printed no terminal verdict")
}

fn trace_matches(
    lines: &[&str],
    verdict: &Verdict,
    verdict_index: usize,
    finished: usize,
) -> Result<bool> {
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
    Ok(match verdict {
        Verdict::Clean => behavior.is_empty() && states.is_empty(),
        Verdict::Violated(_) => {
            behavior.len() == 1
                && verdict_index < behavior[0]
                && behavior[0] < finished
                && states.first().map(|(index, _)| *index) == Some(behavior[0] + 1)
                && states.iter().enumerate().all(|(offset, (index, number))| {
                    behavior[0] < *index && *index < finished && *number == offset + 1
                })
        }
    })
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
