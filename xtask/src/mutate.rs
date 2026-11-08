//! Persistent deliberate defects and the exact checks required to catch them.
//!
//! The cheap gate requires every source pattern to occur exactly once. The
//! complete task applies each mutation in a clean, commit-pinned, journaled
//! workspace, proves the witness class green first, and then requires the
//! mutation's exact check, terminal marker or timeout, boot marker, or host-test
//! verdict to go red. Another failure is inconclusive, and a selected-zero-tests
//! host command proves nothing. Machine entries stay outside CI because each
//! costs a boot.

mod registry;
pub mod workspace;

use anyhow::{Context, Result, bail, ensure};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::paths::Paths;
use crate::{qemu, qualification, sha256, tasks};

pub use self::registry::{Caught, Mutation, NEXT_MUTATION_ID, RETIRED_MUTATION_IDS, mutations};
use self::workspace::{Outcome, Session};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Baselines {
    host: bool,
    api: bool,
    selftest: bool,
    degraded: bool,
}

fn baselines_for(mutations: &[Mutation]) -> Baselines {
    let mut baselines = Baselines::default();
    for mutation in mutations {
        match mutation.reddens {
            Caught::HostTest { .. } => baselines.host = true,
            Caught::ApiAudit(_) => baselines.api = true,
            Caught::Check(_) | Caught::Panic { .. } | Caught::Timeout { .. } => {
                baselines.selftest = true;
            }
            Caught::BootMarker(_) | Caught::DegradedPanic { .. } => {
                baselines.degraded = true;
            }
        }
    }
    baselines
}

fn require_green(label: &str, result: Result<()>) -> Result<()> {
    result.with_context(|| format!("pristine {label} mutation baseline is not green"))
}

fn prove_green_baselines(paths: &Paths, mutations: &[Mutation]) -> Result<()> {
    let baselines = baselines_for(mutations);
    if baselines.host {
        println!("[xtask] proving the pristine host mutation baseline");
        require_green("host", tasks::test(paths))?;
    }
    if baselines.api {
        println!("[xtask] proving the pristine API-audit mutation baseline");
        require_green("API audit", crate::api::check(paths))?;
    }
    if baselines.selftest {
        println!("[xtask] proving the pristine self-test mutation baseline");
        require_green(
            "self-test",
            tasks::test_boot(paths, crate::cargo::Profile::Release),
        )?;
    }
    if baselines.degraded {
        println!("[xtask] proving the pristine degraded mutation baseline");
        require_green(
            "degraded",
            tasks::degraded_check(paths, crate::cargo::Profile::Release),
        )?;
    }
    Ok(())
}

/// Apply each mutation, run its exact machine or host witness, and require red.
///
/// # Errors
///
/// If a pattern no longer matches, if the tree cannot be restored, or if any
/// mutation **survives** — which is the interesting failure and the reason
/// this exists.
pub fn run(paths: &Paths) -> Result<()> {
    let mutations = mutations().copied().collect::<Vec<_>>();
    let registry = registry_fingerprint();
    let qualification_attempt = qualification::begin_mutation(paths, &registry, mutations.len())?;
    let mut session = Session::open(paths, &registry, mutations.len())?;
    if let Some(active) = session.active() {
        let mutation = mutations
            .get(active)
            .with_context(|| format!("journal names absent mutation {active}"))?;
        println!(
            "[xtask] restoring interrupted mutation {}/{} #{}: {}",
            active + 1,
            mutations.len(),
            mutation.id,
            mutation.breaks
        );
        session.recover(active, mutation.file)?;
    }

    let first = session.completed().len();
    if first != 0 {
        println!(
            "[xtask] resuming after {first}/{} durable mutation outcomes",
            mutations.len()
        );
    }
    prove_green_baselines(session.paths(), &mutations[first..])?;
    for (index, mutation) in mutations.iter().enumerate().skip(first) {
        ensure_target_is_rebuilt(mutation)?;
        let original = session.original(mutation.file)?;
        let mutated = apply(&original, mutation)?;

        println!(
            "[xtask] mutate {}/{} #{}: {}",
            index + 1,
            mutations.len(),
            mutation.id,
            mutation.breaks
        );
        session.begin(index)?;
        Session::write(&session, mutation.file, &mutated)?;
        let outcome = match mutation_reports(session.paths(), mutation.reddens) {
            Ok(true) => {
                println!("[xtask]   caught");
                Outcome::Caught
            }
            Ok(false) => {
                println!("[xtask]   SURVIVED");
                Outcome::Survived
            }
            Err(error) => {
                // A different red verdict did not answer this mutation's
                // question, so it is inconclusive rather than a survivor.
                println!("[xtask]   inconclusive: {error}");
                Outcome::Inconclusive(format!("{error:#}"))
            }
        };
        session.finish(index, mutation.file, outcome)?;
    }

    let ids = mutations
        .iter()
        .map(|mutation| mutation.id)
        .collect::<Vec<_>>();
    let outcomes = session.complete(&ids)?;
    let mut survivors = Vec::new();
    let mut inconclusive = Vec::new();
    for (mutation, outcome) in mutations.iter().zip(&outcomes) {
        match outcome {
            Outcome::Caught => {}
            Outcome::Survived => survivors.push(mutation),
            Outcome::Inconclusive(error) => inconclusive.push((mutation, error)),
        }
    }
    if survivors.is_empty() && inconclusive.is_empty() {
        let pass = workspace::current_pass(paths, &registry, mutations.len())?;
        qualification_attempt.pass(&pass)?;
        println!("[xtask] every mutation was caught by the check it names");
        return Ok(());
    }
    for survivor in &survivors {
        println!(
            "[xtask] survived: #{} {} ({})",
            survivor.id, survivor.breaks, survivor.file
        );
    }
    for (mutation, error) in &inconclusive {
        println!(
            "[xtask] inconclusive: #{} {} ({}): {error}",
            mutation.id, mutation.breaks, mutation.file
        );
    }

    match (survivors.len(), inconclusive.len()) {
        (0, count) => bail!(
            "{count} of {} mutations were inconclusive; each needs a valid build and its \
             exact expected verdict",
            mutations.len()
        ),
        (count, 0) => bail!(
            "{count} of {} mutations survived; each is a property nothing checks",
            mutations.len()
        ),
        (survived, inconclusive) => bail!(
            "{survived} of {} mutations survived and {inconclusive} were inconclusive",
            mutations.len()
        ),
    }
}

pub(crate) fn registry_fingerprint() -> String {
    fn field(material: &mut Vec<u8>, value: &str) {
        material.extend_from_slice(&(value.len() as u64).to_le_bytes());
        material.extend_from_slice(value.as_bytes());
    }

    let mut material = Vec::new();
    for mutation in mutations() {
        field(&mut material, &mutation.id.to_string());
        field(&mut material, mutation.file);
        field(&mut material, mutation.find);
        field(&mut material, mutation.replace);
        field(&mut material, mutation.breaks);
        match mutation.reddens {
            Caught::Check(name) => {
                field(&mut material, "check");
                field(&mut material, name);
            }
            Caught::HostTest { package, name } => {
                field(&mut material, "host");
                field(&mut material, package);
                field(&mut material, name);
            }
            Caught::ApiAudit(item) => {
                field(&mut material, "api-audit");
                field(&mut material, item);
            }
            Caught::BootMarker(marker) => {
                field(&mut material, "boot-marker");
                field(&mut material, marker);
            }
            Caught::DegradedPanic { marker, why } => {
                field(&mut material, "degraded-panic");
                field(&mut material, marker);
                field(&mut material, why);
            }
            Caught::Panic { marker, why } => {
                field(&mut material, "panic");
                field(&mut material, marker);
                field(&mut material, why);
            }
            Caught::Timeout { seconds, why } => {
                field(&mut material, "timeout");
                field(&mut material, &seconds.to_string());
                field(&mut material, why);
            }
        }
    }
    sha256::hex(&sha256::digest(&material))
}

/// Refuse source the already-running mutation driver would consume itself.
fn ensure_target_is_rebuilt(mutation: &Mutation) -> Result<()> {
    let rebuilt_witness = matches!(
        mutation.reddens,
        Caught::HostTest {
            package: "xtask",
            ..
        } | Caught::ApiAudit(_)
    );
    if mutation.file.starts_with("xtask/") && !rebuilt_witness {
        bail!(
            "{}: `{}` belongs to the running mutation driver and would not be rebuilt by its witness",
            mutation.file,
            mutation.breaks
        );
    }
    Ok(())
}

/// The mutated text, or an error naming why the pattern is no longer usable.
fn apply(original: &str, mutation: &Mutation) -> Result<String> {
    match original.matches(mutation.find).count() {
        1 => Ok(original.replacen(mutation.find, mutation.replace, 1)),
        0 => bail!(
            "{}: the pattern for `{}` no longer occurs; the mutation tests nothing",
            mutation.file,
            mutation.breaks
        ),
        many => bail!(
            "{}: the pattern for `{}` occurs {many} times; which one it breaks is not decided",
            mutation.file,
            mutation.breaks
        ),
    }
}

/// Report whether this mutation reached its exact named red verdict.
/// A compile refusal or another red verdict is inconclusive, not caught.
fn mutation_reports(paths: &Paths, wanted: Caught) -> Result<bool> {
    if let Caught::HostTest { package, name } = wanted {
        return host_test_reports(paths, package, name);
    }
    if let Caught::BootMarker(marker) = wanted {
        return match tasks::degraded_check(paths, crate::cargo::Profile::Release) {
            Ok(()) => Ok(false),
            Err(error) => {
                let expected = format!("without reporting {marker:?}");
                if format!("{error:#}").contains(&expected) {
                    Ok(true)
                } else {
                    Err(error)
                }
            }
        };
    }
    if matches!(wanted, Caught::DegradedPanic { .. }) {
        return qualify_degraded_mutation(wanted, || {
            tasks::observe_no_x2apic_boot(paths, crate::cargo::Profile::Release)
        });
    }
    if let Caught::ApiAudit(item) = wanted {
        return match crate::api::check(paths) {
            Ok(()) => Ok(false),
            Err(error) => {
                let report = format!("{error:#}");
                if crate::api::report_names_exact_dead_item(&report, item) {
                    Ok(true)
                } else {
                    Err(error)
                }
            }
        };
    }
    let timeout = machine_observation_timeout(wanted);
    qualify_machine_mutation(wanted, || {
        tasks::observe_test_boot(paths, crate::cargo::Profile::Release, timeout)
    })
}

const MACHINE_OBSERVATIONS: usize = 2;

const fn machine_observation_timeout(wanted: Caught) -> Duration {
    match wanted {
        Caught::Timeout { seconds, .. } => Duration::from_secs(seconds),
        _ => tasks::MUTATION_SELFTEST_TIMEOUT,
    }
}

fn qualify_machine_mutation(
    wanted: Caught,
    mut observe: impl FnMut() -> Result<qemu::SelftestOutcome>,
) -> Result<bool> {
    let mut first_error = None;
    for attempt in 1..=MACHINE_OBSERVATIONS {
        let assessment = match observe() {
            Ok(observed) => assess_selftest(wanted, observed),
            Err(error) => assess_machine_error(wanted, error),
        };
        match assessment {
            Ok(caught) => return Ok(caught),
            Err(error) if attempt < MACHINE_OBSERVATIONS => {
                let report = format!("{error:#}");
                eprintln!("[xtask]   machine observation inconclusive; retrying once: {report}");
                first_error = Some(report);
            }
            Err(error) => {
                if let Some(first) = first_error {
                    bail!(
                        "machine mutation remained inconclusive after {MACHINE_OBSERVATIONS} observations; first: {first}; retry: {error:#}"
                    );
                }
                return Err(error);
            }
        }
    }
    unreachable!("the bounded observation loop returns on every outcome")
}

fn assess_machine_error(wanted: Caught, error: anyhow::Error) -> Result<bool> {
    let Caught::Timeout { seconds, .. } = wanted else {
        return Err(error);
    };
    let expected = format!("self-test did not finish within {seconds} seconds");
    if error.to_string() == expected {
        Ok(true)
    } else {
        bail!("mutation expected timeout {expected:?}, but observed: {error:#}")
    }
}

fn qualify_degraded_mutation(
    wanted: Caught,
    mut observe: impl FnMut() -> Result<qemu::DegradedOutcome>,
) -> Result<bool> {
    let mut first_error = None;
    for attempt in 1..=MACHINE_OBSERVATIONS {
        match observe().and_then(|observed| assess_degraded_panic(wanted, observed)) {
            Ok(caught) => return Ok(caught),
            Err(error) if attempt < MACHINE_OBSERVATIONS => {
                let report = format!("{error:#}");
                eprintln!("[xtask]   degraded observation inconclusive; retrying once: {report}");
                first_error = Some(report);
            }
            Err(error) => {
                if let Some(first) = first_error {
                    bail!(
                        "degraded mutation remained inconclusive after {MACHINE_OBSERVATIONS} observations; first: {first}; retry: {error:#}"
                    );
                }
                return Err(error);
            }
        }
    }
    unreachable!("the bounded observation loop returns on every outcome")
}

fn assess_degraded_panic(wanted: Caught, observed: qemu::DegradedOutcome) -> Result<bool> {
    let Caught::DegradedPanic { marker, .. } = wanted else {
        unreachable!("only a degraded-panic witness reaches degraded assessment")
    };
    match observed {
        qemu::DegradedOutcome::Settled => Ok(false),
        qemu::DegradedOutcome::Panicked {
            no_x2apic_diagnosed: false,
            transcript,
        } => bail!(
            "mutation expected a prior no-x2APIC diagnosis before panic {marker:?}, but observed: {transcript}"
        ),
        qemu::DegradedOutcome::Panicked { transcript, .. } if transcript.contains(marker) => {
            Ok(true)
        }
        qemu::DegradedOutcome::Panicked { transcript, .. } => {
            bail!("mutation expected degraded panic marker {marker:?}, but observed: {transcript}")
        }
    }
}

fn assess_selftest(wanted: Caught, observed: qemu::SelftestOutcome) -> Result<bool> {
    match (wanted, observed) {
        (_, qemu::SelftestOutcome::Passed) => Ok(false),
        (Caught::Check(name), qemu::SelftestOutcome::Failed(failed))
            if failed.iter().any(|actual| actual == name) =>
        {
            Ok(true)
        }
        (Caught::Check(name), qemu::SelftestOutcome::Failed(failed)) => bail!(
            "mutation expected check {name:?}, but different checks failed: {}",
            failed.join("; ")
        ),
        (Caught::Check(name), qemu::SelftestOutcome::Panicked { failed, .. })
            if failed.iter().any(|actual| actual == name) =>
        {
            Ok(true)
        }
        (Caught::Check(name), qemu::SelftestOutcome::Panicked { transcript, failed }) => {
            let earlier = if failed.is_empty() {
                "none".to_string()
            } else {
                failed.join("; ")
            };
            bail!(
                "mutation expected check {name:?}, but earlier checks were [{earlier}] before the kernel panicked: {transcript}"
            )
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Panicked { transcript, .. })
            if transcript.contains(marker) =>
        {
            Ok(true)
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Panicked { transcript, .. }) => {
            bail!("mutation expected panic marker {marker:?}, but observed: {transcript}")
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Failed(failed)) => bail!(
            "mutation expected panic marker {marker:?}, but checks failed: {}",
            failed.join("; ")
        ),
        (Caught::Timeout { seconds, .. }, qemu::SelftestOutcome::Failed(failed)) => bail!(
            "mutation expected a {seconds}-second timeout, but checks failed: {}",
            failed.join("; ")
        ),
        (Caught::Timeout { seconds, .. }, qemu::SelftestOutcome::Panicked { transcript, .. }) => {
            bail!(
                "mutation expected a {seconds}-second timeout, but the kernel panicked: {transcript}"
            )
        }
        (
            Caught::HostTest { .. }
            | Caught::ApiAudit(_)
            | Caught::BootMarker(_)
            | Caught::DegradedPanic { .. },
            _,
        ) => {
            unreachable!("other witnesses return before self-test assessment")
        }
    }
}

const HOST_TEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);

static HOST_TEST_TEMPORARIES: AtomicU64 = AtomicU64::new(0);

/// A caught mutation can strand fixed-name scratch below the temporary
/// directory, which would redden every later witness that reserves it, so each
/// host witness gets a new empty temporary directory of its own.
fn fresh_host_test_temporary(paths: &Paths) -> Result<PathBuf> {
    let target = paths.target_dir();
    let parent = target.join("host-temp");
    let parent = workspace::prepare_real_child(target, &parent, "host mutation temporary parent")?;
    loop {
        let ordinal = HOST_TEST_TEMPORARIES.fetch_add(1, Ordering::Relaxed);
        let child = parent.join(format!("{}-{ordinal}", std::process::id()));
        match std::fs::create_dir(&child) {
            Ok(()) => {
                return workspace::prepare_real_child(
                    &parent,
                    &child,
                    "host mutation temporary directory",
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| format!("creating {}", child.display()));
            }
        }
    }
}

fn host_test_reports(paths: &Paths, package: &str, name: &str) -> Result<bool> {
    host_test_reports_with(paths, package, name, |command, timeout, label| {
        crate::bounded_process::output(command, timeout, label)
    })
}

fn host_test_reports_with<F>(paths: &Paths, package: &str, name: &str, run: F) -> Result<bool>
where
    F: FnOnce(Command, Duration, &str) -> Result<Output>,
{
    let temporary = crate::cargo::child_process_path(&fresh_host_test_temporary(paths)?);
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(paths.root())
        .env(
            "CARGO_TARGET_DIR",
            crate::cargo::child_process_path(paths.target_dir()),
        )
        .env("TEMP", &temporary)
        .env("TMP", &temporary)
        .env("TMPDIR", &temporary)
        .args(["test", "--package", package, name, "--", "--exact"]);
    let label = format!("host mutation test {package}::{name}");
    let output = run(command, HOST_TEST_TIMEOUT, &label)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assess_host_test(
        name,
        output.status.success(),
        &stdout,
        &stderr,
        &output.status.to_string(),
    )
}

fn assess_host_test(
    name: &str,
    success: bool,
    stdout: &str,
    stderr: &str,
    status: &str,
) -> Result<bool> {
    let failed = format!("test {name} ... FAILED");
    let failed = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|line| *line == failed)
        .count();
    ensure!(
        failed <= 1,
        "host mutation printed duplicate failed verdicts"
    );
    if failed == 1 {
        ensure!(
            !success,
            "host mutation printed a failed verdict but Cargo exited successfully"
        );
        return Ok(true);
    }
    if success {
        let passed = format!("test {name} ... ok");
        let passed = stdout
            .lines()
            .chain(stderr.lines())
            .filter(|line| *line == passed)
            .count();
        if passed == 1 {
            return Ok(false);
        }
        bail!("host mutation selected no exact test `{name}`");
    }
    bail!("host mutation did not reach `{name}`; cargo exited {status}\n{stdout}{stderr}")
}

#[cfg(test)]
mod tests;
