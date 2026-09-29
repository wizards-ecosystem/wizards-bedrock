//! Source-bound import and replay checks for the independent `xtask` grade.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::cli::MutantsGradeAction;
use crate::json::{
    JsonValue, json_array, json_field, json_object, json_string, json_u64, parse_json,
    require_json_fields,
};
use crate::paths::Paths;
use crate::qualification::{self, ExternalArtifact, Kind, SourceIdentity};
use crate::sha256;

const VERSION: &str = "27.1.0";
const NEXTEST_VERSION: &str = "0.9.145";
const NEXTEST_COMMIT: &str = "00af4550ec3b3b9f0e574b897b06acb95d325ba2";
const NEXTEST_URL: &str = "https://get.nexte.st/0.9.145/windows";
const NEXTEST_ARCHIVE_SHA256: &str =
    "5bc4b6789103e9834f596eff2b290cfb5b2b69e5413c78bbaa5acc835ac5baa0";
const NEXTEST_EXECUTABLE_SHA256: &str =
    "3fca621fe9e9a5bb7461367ea74fba4c85de824fdb77519b5678fbf91f40c86f";
const COMMAND: &str = "cargo mutants -p xtask --output <source-bound-directory> --jobs 1 --jobserver-tasks 1 --test-tool nextest --timeout-multiplier 3 --no-shuffle";
/// The directory cargo-mutants creates inside its `--output` argument.
const CARGO_MUTANTS_OUTPUT: &str = "mutants.out";
/// The reviewed crates.io `cargo-mutants 27.1.0` executable (D-342).
const CARGO_MUTANTS_EXECUTABLE_SHA256: &str =
    "4ef442752564625eef9786739a1d57cc692263fca071dd16751f45ba2e322d9c";
const INVENTORY_COMMAND: &str = "cargo mutants -p xtask --list --json --diff --no-times";
const INVENTORY_ARGUMENTS: [&str; 7] = [
    "mutants",
    "-p",
    "xtask",
    "--list",
    "--json",
    "--diff",
    "--no-times",
];
const INVENTORY_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DOCTEST_COMMAND: &str = "cargo test -p xtask --doc --locked";
const DOCTEST_ARGUMENTS: [&str; 5] = ["test", "-p", "xtask", "--doc", "--locked"];
const DOCTEST_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const REQUEST: &str = "request";
const DOCTEST: &str = "doctest.transcript";
const ADJUDICATIONS: &str = "adjudications.json";
const INVENTORY: &str = "inventory.json";
const MUTANTS: &str = "mutants.json";
const OUTCOMES: &str = "outcomes.json";
const MAX_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ADJUDICATION_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECEIPT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Grade {
    total: u64,
    caught: u64,
    missed: u64,
    timeout: u64,
    unviable: u64,
    adjudicated: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolIdentity {
    cargo_mutants_path: String,
    cargo_mutants_bytes: u64,
    cargo_mutants_sha256: String,
    nextest_path: String,
    nextest_bytes: u64,
}

impl Grade {
    fn fields(&self, tools: &ToolIdentity) -> Vec<(String, String)> {
        [
            ("grade.adjudicated", self.adjudicated),
            ("grade.caught", self.caught),
            ("grade.missed", self.missed),
            ("grade.timeout", self.timeout),
            ("grade.total", self.total),
            ("grade.unviable", self.unviable),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .chain([
            (
                "execution.authority".to_string(),
                "external-operator-v1".to_string(),
            ),
            (
                "tool.cargo-mutants.bytes".to_string(),
                tools.cargo_mutants_bytes.to_string(),
            ),
            (
                "tool.cargo-mutants.sha256".to_string(),
                tools.cargo_mutants_sha256.clone(),
            ),
            (
                "tool.cargo-mutants.version".to_string(),
                VERSION.to_string(),
            ),
            (
                "tool.cargo-nextest.archive.sha256".to_string(),
                NEXTEST_ARCHIVE_SHA256.to_string(),
            ),
            (
                "tool.cargo-nextest.bytes".to_string(),
                tools.nextest_bytes.to_string(),
            ),
            (
                "tool.cargo-nextest.commit".to_string(),
                NEXTEST_COMMIT.to_string(),
            ),
            (
                "tool.cargo-nextest.sha256".to_string(),
                NEXTEST_EXECUTABLE_SHA256.to_string(),
            ),
            (
                "tool.cargo-nextest.url".to_string(),
                NEXTEST_URL.to_string(),
            ),
            (
                "tool.cargo-nextest.version".to_string(),
                NEXTEST_VERSION.to_string(),
            ),
        ])
        .collect()
    }
}

pub(crate) fn run(paths: &Paths, action: MutantsGradeAction) -> Result<()> {
    match action {
        MutantsGradeAction::Begin => begin(paths),
        MutantsGradeAction::Seal(directory) => seal(paths, &directory),
    }
}

fn begin(paths: &Paths) -> Result<()> {
    let attempt = qualification::begin_external(paths, Kind::CargoMutantsGrade, false)?;
    let tools = discover_tools(paths)?;
    let request = render_request(attempt.source(), &tools)?;
    attempt.write_mutants_request(&request)?;
    println!(
        "[xtask] cargo-mutants grade request ready at {}",
        attempt.directory().join(REQUEST).display()
    );
    Ok(())
}

fn seal(paths: &Paths, source_directory: &Path) -> Result<()> {
    let attempt = qualification::begin_external(paths, Kind::CargoMutantsGrade, true)?;
    let root = canonical_directory(source_directory, "cargo-mutants grade directory")?;
    let records = canonical_directory(
        &paths.target_dir().join("qualification"),
        "qualification records",
    )?;
    require_outside_records(&root, &records)?;
    let request = read(
        &attempt.directory().join(REQUEST),
        MAX_RECEIPT_BYTES,
        "cargo-mutants run request",
    )?;
    let output = canonical_directory(&output_directory(&root), "cargo-mutants output directory")?;
    let adjudications = read(
        &root.join(ADJUDICATIONS),
        MAX_ADJUDICATION_BYTES,
        "cargo-mutants adjudications",
    )?;
    reject_byte_order_mark(&adjudications, "cargo-mutants adjudications")?;
    let mutants = read(
        &output.join(MUTANTS),
        MAX_JSON_BYTES,
        "cargo-mutants candidate inventory",
    )?;
    let outcomes = read(
        &output.join(OUTCOMES),
        MAX_JSON_BYTES,
        "cargo-mutants outcomes",
    )?;

    let tools = verify_request(&request, attempt.source())?;
    ensure!(
        discover_tools(paths)? == tools,
        "cargo-mutants or cargo-nextest changed between request and seal"
    );
    // The completeness witness and the doctest proof come from this checkout,
    // never from files the operator supplies beside the run.
    let inventory = capture_inventory(&tools, paths.root())?;
    let doctest = capture_doctest(paths)?;
    verify_doctest(&doctest)?;
    let candidates = parse_candidates(&mutants, "cargo-mutants run inventory")?;
    let independent = parse_candidates(&inventory, "independent cargo-mutants inventory")?;
    require_same_inventory(&candidates, &independent)?;
    let (mut grade, noncaught) = parse_outcomes(&outcomes, &candidates)?;
    grade.adjudicated = verify_adjudications(&adjudications, &noncaught)?;
    let fields = grade.fields(&tools);
    attempt.seal(
        &fields,
        vec![
            ExternalArtifact::MutantsRequest(request),
            ExternalArtifact::MutantsInventory(inventory),
            ExternalArtifact::MutantsCandidates(mutants),
            ExternalArtifact::MutantsOutcomes(outcomes),
            ExternalArtifact::MutantsAdjudications(adjudications),
            ExternalArtifact::MutantsDoctest(doctest),
        ],
    )
}

pub(crate) fn verify_sealed(
    directory: &Path,
    source: &SourceIdentity,
) -> Result<Vec<(String, String)>> {
    let request = read(
        &directory.join(REQUEST),
        MAX_RECEIPT_BYTES,
        "sealed cargo-mutants request",
    )?;
    let inventory = read(
        &directory.join(INVENTORY),
        MAX_JSON_BYTES,
        "sealed independent mutant inventory",
    )?;
    let doctest = read(
        &directory.join(DOCTEST),
        MAX_RECEIPT_BYTES,
        "sealed doctest transcript",
    )?;
    let adjudications = read(
        &directory.join(ADJUDICATIONS),
        MAX_ADJUDICATION_BYTES,
        "sealed adjudications",
    )?;
    let mutants = read(
        &directory.join(MUTANTS),
        MAX_JSON_BYTES,
        "sealed mutant inventory",
    )?;
    let outcomes = read(
        &directory.join(OUTCOMES),
        MAX_JSON_BYTES,
        "sealed mutant outcomes",
    )?;
    let tools = verify_request(&request, source)?;
    verify_doctest(&doctest)?;
    let candidates = parse_candidates(&mutants, "sealed run mutant inventory")?;
    let independent = parse_candidates(&inventory, "sealed independent mutant inventory")?;
    require_same_inventory(&candidates, &independent)?;
    let (mut grade, noncaught) = parse_outcomes(&outcomes, &candidates)?;
    grade.adjudicated = verify_adjudications(&adjudications, &noncaught)?;
    Ok(grade.fields(&tools))
}

fn read(path: &Path, maximum: u64, label: &str) -> Result<Vec<u8>> {
    qualification::read_bounded_file(path, maximum, label)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    crate::mutation_workspace::canonical_existing_real_directory(path, label)
}

fn parse_candidates(bytes: &[u8], label: &str) -> Result<BTreeMap<String, String>> {
    let value = parse_json(bytes, label)?;
    let candidates = json_array(&value, label)?;
    ensure!(
        !candidates.is_empty(),
        "cargo-mutants candidate inventory is empty"
    );
    let mut observed = BTreeMap::new();
    for candidate in candidates {
        let candidate = json_object(candidate, "cargo-mutants candidate")?;
        require_json_fields(
            candidate,
            &[
                "diff",
                "file",
                "function",
                "genre",
                "name",
                "package",
                "replacement",
                "span",
            ],
            "cargo-mutants candidate",
        )?;
        ensure!(
            json_string(
                json_field(candidate, "package", "cargo-mutants candidate")?,
                "candidate package"
            )? == "xtask",
            "cargo-mutants candidate belongs to another package"
        );
        let name = json_string(
            json_field(candidate, "name", "cargo-mutants candidate")?,
            "candidate name",
        )?;
        let diff = json_string(
            json_field(candidate, "diff", "cargo-mutants candidate")?,
            "candidate diff",
        )?;
        ensure!(
            !name.is_empty()
                && !diff.is_empty()
                && observed
                    .insert(name.to_string(), diff.to_string())
                    .is_none(),
            "cargo-mutants candidate identities are empty or duplicated"
        );
    }
    Ok(observed)
}

fn require_same_inventory(
    run: &BTreeMap<String, String>,
    independent: &BTreeMap<String, String>,
) -> Result<()> {
    ensure!(
        run == independent,
        "independent cargo-mutants inventory differs from the run candidate set"
    );
    Ok(())
}

fn parse_outcomes(
    bytes: &[u8],
    candidates: &BTreeMap<String, String>,
) -> Result<(Grade, BTreeMap<String, String>)> {
    let value = parse_json(bytes, "cargo-mutants outcomes")?;
    let root = json_object(&value, "cargo-mutants outcomes")?;
    require_json_fields(
        root,
        &[
            "outcomes",
            "total_mutants",
            "missed",
            "caught",
            "timeout",
            "unviable",
            "success",
            "start_time",
            "end_time",
            "cargo_mutants_version",
        ],
        "cargo-mutants outcomes",
    )?;
    ensure!(
        json_string(
            json_field(root, "cargo_mutants_version", "outcomes")?,
            "cargo-mutants version"
        )? == VERSION,
        "cargo-mutants version differs from D-324"
    );
    let start = json_string(
        json_field(root, "start_time", "outcomes")?,
        "cargo-mutants start time",
    )?;
    let end = json_string(
        json_field(root, "end_time", "outcomes")?,
        "cargo-mutants end time",
    )?;
    let start = parse_utc_timestamp(start, "cargo-mutants start time")?;
    let end = parse_utc_timestamp(end, "cargo-mutants end time")?;
    ensure!(start < end, "cargo-mutants completion precedes its start");
    ensure!(
        json_u64(
            json_field(root, "success", "outcomes")?,
            "cargo-mutants success count"
        )? == 0,
        "cargo-mutants reports unclassified successful mutants"
    );

    let mut baseline = 0u64;
    let mut observed = BTreeSet::new();
    let mut noncaught = BTreeMap::new();
    let mut caught = 0u64;
    let mut missed = 0u64;
    let mut timeout = 0u64;
    let mut unviable = 0u64;
    for outcome in json_array(
        json_field(root, "outcomes", "outcomes")?,
        "cargo-mutants outcome list",
    )? {
        let outcome = json_object(outcome, "cargo-mutants outcome")?;
        require_json_fields(
            outcome,
            &[
                "scenario",
                "summary",
                "log_path",
                "diff_path",
                "phase_results",
            ],
            "cargo-mutants outcome",
        )?;
        let summary = json_string(
            json_field(outcome, "summary", "outcome")?,
            "outcome summary",
        )?;
        let log = json_string(
            json_field(outcome, "log_path", "outcome")?,
            "outcome log path",
        )?;
        require_relative_path(log)?;
        match json_field(outcome, "scenario", "outcome")? {
            JsonValue::String(value) => {
                ensure!(
                    value == "Baseline" && summary == "Success",
                    "cargo-mutants pristine baseline did not pass"
                );
                ensure!(
                    matches!(
                        json_field(outcome, "diff_path", "outcome")?,
                        JsonValue::Null
                    ),
                    "cargo-mutants baseline has a diff"
                );
                require_nextest_phases(json_field(outcome, "phase_results", "outcome")?, summary)?;
                baseline += 1;
            }
            JsonValue::Object(scenario) => {
                require_json_fields(scenario, &["Mutant"], "mutant scenario")?;
                let mutant = json_object(
                    json_field(scenario, "Mutant", "mutant scenario")?,
                    "mutant scenario",
                )?;
                require_json_fields(
                    mutant,
                    &[
                        "name",
                        "package",
                        "file",
                        "function",
                        "span",
                        "replacement",
                        "genre",
                    ],
                    "mutant scenario",
                )?;
                ensure!(
                    json_string(
                        json_field(mutant, "package", "mutant scenario")?,
                        "mutant package"
                    )? == "xtask",
                    "cargo-mutants outcome belongs to another package"
                );
                let name = json_string(
                    json_field(mutant, "name", "mutant scenario")?,
                    "mutant name",
                )?;
                ensure!(
                    candidates.contains_key(name) && observed.insert(name.to_string()),
                    "cargo-mutants outcome has an unknown or duplicate candidate"
                );
                let diff = json_string(
                    json_field(outcome, "diff_path", "outcome")?,
                    "outcome diff path",
                )?;
                require_relative_path(diff)?;
                require_nextest_phases(json_field(outcome, "phase_results", "outcome")?, summary)?;
                match summary {
                    "CaughtMutant" => caught += 1,
                    "MissedMutant" => {
                        missed += 1;
                        noncaught.insert(name.to_string(), summary.to_string());
                    }
                    "Timeout" => {
                        timeout += 1;
                        noncaught.insert(name.to_string(), summary.to_string());
                    }
                    "Unviable" => {
                        unviable += 1;
                        noncaught.insert(name.to_string(), summary.to_string());
                    }
                    "Failure" | "Success" => {
                        bail!("cargo-mutants grade contains an unclassified `{summary}` outcome")
                    }
                    other => bail!("cargo-mutants grade contains unknown outcome `{other}`"),
                }
            }
            _ => bail!("cargo-mutants outcome scenario is malformed"),
        }
    }
    ensure!(
        baseline == 1,
        "cargo-mutants grade needs exactly one passing pristine baseline"
    );
    ensure!(
        observed == candidates.keys().cloned().collect(),
        "cargo-mutants candidate and outcome sets differ"
    );
    let total = candidates.len() as u64;
    for (key, actual) in [
        ("total_mutants", total),
        ("caught", caught),
        ("missed", missed),
        ("timeout", timeout),
        ("unviable", unviable),
    ] {
        ensure!(
            json_u64(json_field(root, key, "outcomes")?, key)? == actual,
            "cargo-mutants `{key}` counter differs from its outcomes"
        );
    }
    Ok((
        Grade {
            total,
            caught,
            missed,
            timeout,
            unviable,
            adjudicated: 0,
        },
        noncaught,
    ))
}

fn require_nextest_phases(value: &JsonValue, summary: &str) -> Result<()> {
    let phases = json_array(value, "cargo-mutants phase results")?;
    let expected: &[(&str, &str)] = match summary {
        "Success" | "MissedMutant" => &[("Build", "Success"), ("Test", "Success")],
        "CaughtMutant" => &[("Build", "Success"), ("Test", "Failure")],
        "Unviable" => &[("Build", "Failure")],
        "Timeout" if phases.len() == 1 => &[("Build", "Timeout")],
        "Timeout" => &[("Build", "Success"), ("Test", "Timeout")],
        _ => bail!("cargo-mutants phase summary is unsupported"),
    };
    ensure!(
        phases.len() == expected.len(),
        "cargo-mutants phase count differs from its outcome"
    );
    for (phase, (expected_phase, expected_status)) in phases.iter().zip(expected) {
        let phase = json_object(phase, "cargo-mutants phase")?;
        require_json_fields(
            phase,
            &["phase", "duration", "process_status", "argv"],
            "cargo-mutants phase",
        )?;
        ensure!(
            json_string(
                json_field(phase, "phase", "cargo-mutants phase")?,
                "cargo-mutants phase name"
            )? == *expected_phase,
            "cargo-mutants phase order differs from its outcome"
        );
        let JsonValue::Number(duration) = json_field(phase, "duration", "cargo-mutants phase")?
        else {
            bail!("cargo-mutants phase duration is not a number");
        };
        let duration = duration
            .parse::<f64>()
            .context("cargo-mutants phase duration is malformed")?;
        ensure!(
            duration.is_finite() && duration >= 0.0,
            "cargo-mutants phase duration is invalid"
        );
        require_process_status(
            json_field(phase, "process_status", "cargo-mutants phase")?,
            expected_status,
        )?;
        let argv: Vec<_> = json_array(
            json_field(phase, "argv", "cargo-mutants phase")?,
            "cargo-mutants phase argv",
        )?
        .iter()
        .map(|arg| json_string(arg, "cargo-mutants phase argument"))
        .collect::<Result<_>>()?;
        let arguments = argv
            .get(1..)
            .context("cargo-mutants phase has no executable")?;
        let expected_arguments: &[&str] = if *expected_phase == "Build" {
            &[
                "nextest",
                "run",
                "--no-run",
                "--verbose",
                "--package=xtask@0.1.0",
            ]
        } else {
            &["nextest", "run", "--verbose", "--package=xtask@0.1.0"]
        };
        ensure!(
            !argv[0].is_empty() && arguments == expected_arguments,
            "cargo-mutants phase did not use the exact D-324 nextest command"
        );
    }
    Ok(())
}

fn require_process_status(value: &JsonValue, expected: &str) -> Result<()> {
    match expected {
        "Success" | "Timeout" => ensure!(
            matches!(value, JsonValue::String(status) if status == expected),
            "cargo-mutants phase status differs from its outcome"
        ),
        "Failure" => {
            let status = json_object(value, "cargo-mutants failure status")?;
            require_json_fields(status, &["Failure"], "cargo-mutants failure status")?;
            let JsonValue::Number(code) =
                json_field(status, "Failure", "cargo-mutants failure status")?
            else {
                bail!("cargo-mutants failure exit code is not a number");
            };
            let parsed = code
                .parse::<i32>()
                .context("cargo-mutants failure exit code is malformed")?;
            ensure!(
                parsed != 0 && parsed.to_string() == *code,
                "cargo-mutants failure exit code is zero or noncanonical"
            );
        }
        _ => unreachable!("phase status is selected from a closed set"),
    }
    Ok(())
}

fn verify_adjudications(bytes: &[u8], expected: &BTreeMap<String, String>) -> Result<u64> {
    let value = parse_json(bytes, "cargo-mutants adjudications")?;
    let root = json_object(&value, "cargo-mutants adjudications")?;
    require_json_fields(root, &["format", "outcomes"], "cargo-mutants adjudications")?;
    ensure!(
        json_string(
            json_field(root, "format", "adjudications")?,
            "adjudication format"
        )? == "wizard-cargo-mutants-adjudications-v1",
        "unsupported cargo-mutants adjudication format"
    );
    let mut observed = BTreeMap::new();
    for row in json_array(
        json_field(root, "outcomes", "adjudications")?,
        "adjudication rows",
    )? {
        let row = json_object(row, "cargo-mutants adjudication")?;
        require_json_fields(
            row,
            &["name", "summary", "disposition", "rationale"],
            "cargo-mutants adjudication",
        )?;
        let name = json_string(
            json_field(row, "name", "adjudication")?,
            "adjudication name",
        )?;
        let summary = json_string(
            json_field(row, "summary", "adjudication")?,
            "adjudication summary",
        )?;
        let disposition = json_string(
            json_field(row, "disposition", "adjudication")?,
            "adjudication disposition",
        )?;
        let allowed = match summary {
            "MissedMutant" => disposition == "equivalent",
            "Unviable" => matches!(disposition, "compiler-rejected" | "equivalent"),
            "Timeout" => matches!(disposition, "bounded-nontermination" | "equivalent"),
            _ => false,
        };
        ensure!(allowed, "non-caught mutant has an invalid disposition");
        ensure!(
            !json_string(
                json_field(row, "rationale", "adjudication")?,
                "adjudication rationale"
            )?
            .trim()
            .is_empty(),
            "cargo-mutants adjudication has no rationale"
        );
        ensure!(
            observed
                .insert(name.to_string(), summary.to_string())
                .is_none(),
            "cargo-mutants adjudication is duplicated"
        );
    }
    ensure!(
        observed == *expected,
        "cargo-mutants non-caught outcomes and adjudications differ"
    );
    Ok(observed.len() as u64)
}

pub(crate) fn parse_utc_timestamp(
    value: &str,
    label: &str,
) -> Result<(u32, u32, u32, u32, u32, u32, u32)> {
    let body = value
        .strip_suffix('Z')
        .context("UTC timestamp has no Z suffix")?;
    let (whole, fraction) = body
        .split_once('.')
        .map_or((body, None), |(whole, fraction)| (whole, Some(fraction)));
    ensure!(
        whole.len() == 19
            && whole.bytes().enumerate().all(|(index, byte)| match index {
                4 | 7 => byte == b'-',
                10 => byte == b'T',
                13 | 16 => byte == b':',
                _ => byte.is_ascii_digit(),
            })
            && fraction.is_none_or(|fraction| {
                !fraction.is_empty()
                    && fraction.len() <= 9
                    && fraction.bytes().all(|byte| byte.is_ascii_digit())
            }),
        "{label} is not a canonical UTC timestamp"
    );
    let number = |start: usize, end: usize| -> Result<u32> {
        whole[start..end]
            .parse::<u32>()
            .with_context(|| format!("{label} is malformed"))
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    ensure!(
        year != 0 && day != 0 && day <= days && hour < 24 && minute < 60 && second < 60,
        "{label} is outside the UTC calendar"
    );
    let nanos = fraction
        .map(|fraction| {
            let mut padded = fraction.to_string();
            padded.extend(std::iter::repeat_n('0', 9 - fraction.len()));
            padded
                .parse::<u32>()
                .with_context(|| format!("{label} has a malformed decimal fraction"))
        })
        .transpose()?
        .unwrap_or(0);
    Ok((year, month, day, hour, minute, second, nanos))
}

fn require_relative_path(path: &str) -> Result<()> {
    let path = Path::new(path);
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "cargo-mutants artifact path is unsafe"
    );
    Ok(())
}

fn render_request(source: &SourceIdentity, tools: &ToolIdentity) -> Result<Vec<u8>> {
    let nextest_parent = Path::new(&tools.nextest_path)
        .parent()
        .context("cargo-nextest path has no parent")?;
    let body = format!(
        "wizard-cargo-mutants-request-v2\nsource.commit={}\nsource.tree={}\ncommand={}\nenvironment.CARGO_BUILD_JOBS=1\nenvironment.NEXTEST_TEST_THREADS=1\nenvironment.PATH.nextest-first={}\ninventory.command={}\ninventory.runner=xtask-seal\ndoctest.command={}\ndoctest.runner=xtask-seal\nexecution.authority=external-operator\ntool.cargo-mutants.bytes={}\ntool.cargo-mutants.path={}\ntool.cargo-mutants.sha256={}\ntool.cargo-mutants.version={}\ntool.cargo-nextest.archive.sha256={}\ntool.cargo-nextest.bytes={}\ntool.cargo-nextest.commit={}\ntool.cargo-nextest.path={}\ntool.cargo-nextest.sha256={}\ntool.cargo-nextest.url={}\ntool.cargo-nextest.version={}\n",
        source.commit,
        source.tree,
        COMMAND,
        nextest_parent.display(),
        INVENTORY_COMMAND,
        DOCTEST_COMMAND,
        tools.cargo_mutants_bytes,
        tools.cargo_mutants_path,
        tools.cargo_mutants_sha256,
        VERSION,
        NEXTEST_ARCHIVE_SHA256,
        tools.nextest_bytes,
        NEXTEST_COMMIT,
        tools.nextest_path,
        NEXTEST_EXECUTABLE_SHA256,
        NEXTEST_URL,
        NEXTEST_VERSION,
    );
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    Ok(format!("{body}checksum.sha256={checksum}\n").into_bytes())
}

fn verify_request(bytes: &[u8], source: &SourceIdentity) -> Result<ToolIdentity> {
    let text = std::str::from_utf8(bytes).context("cargo-mutants request is not UTF-8")?;
    let without_lf = text
        .strip_suffix('\n')
        .context("cargo-mutants request has no terminal LF")?;
    let (body, checksum) = without_lf
        .rsplit_once("checksum.sha256=")
        .context("cargo-mutants request has no checksum")?;
    let mut canonical_body = body.to_string();
    ensure!(
        canonical_body.ends_with('\n') && !checksum.contains('\n'),
        "cargo-mutants request checksum is malformed"
    );
    ensure!(
        sha256::hex(&sha256::digest(canonical_body.as_bytes())) == checksum,
        "cargo-mutants request checksum differs"
    );
    canonical_body.pop();
    let mut lines = canonical_body.lines();
    ensure!(
        lines.next() == Some("wizard-cargo-mutants-request-v2"),
        "unsupported cargo-mutants request"
    );
    let mut fields = BTreeMap::new();
    for line in lines {
        let (key, value) = line
            .split_once('=')
            .context("cargo-mutants receipt line has no value")?;
        ensure!(
            fields.insert(key, value).is_none(),
            "cargo-mutants receipt field is duplicated"
        );
    }
    let expected = BTreeMap::from([
        ("source.commit", source.commit.as_str()),
        ("source.tree", source.tree.as_str()),
        ("command", COMMAND),
        ("environment.CARGO_BUILD_JOBS", "1"),
        ("environment.NEXTEST_TEST_THREADS", "1"),
        ("inventory.command", INVENTORY_COMMAND),
        ("inventory.runner", "xtask-seal"),
        ("doctest.command", DOCTEST_COMMAND),
        ("doctest.runner", "xtask-seal"),
        ("execution.authority", "external-operator"),
        ("tool.cargo-mutants.sha256", CARGO_MUTANTS_EXECUTABLE_SHA256),
        ("tool.cargo-mutants.version", VERSION),
        ("tool.cargo-nextest.archive.sha256", NEXTEST_ARCHIVE_SHA256),
        ("tool.cargo-nextest.version", NEXTEST_VERSION),
        ("tool.cargo-nextest.commit", NEXTEST_COMMIT),
        ("tool.cargo-nextest.url", NEXTEST_URL),
        ("tool.cargo-nextest.sha256", NEXTEST_EXECUTABLE_SHA256),
    ]);
    for (key, value) in expected {
        ensure!(
            fields.remove(key) == Some(value),
            "cargo-mutants request `{key}` differs from D-324"
        );
    }
    let cargo_mutants_bytes = canonical_u64(
        fields
            .remove("tool.cargo-mutants.bytes")
            .context("cargo-mutants request has no grader length")?,
        "cargo-mutants length",
    )?;
    let cargo_mutants_path = fields
        .remove("tool.cargo-mutants.path")
        .context("cargo-mutants request has no grader path")?
        .to_string();
    let nextest_bytes = canonical_u64(
        fields
            .remove("tool.cargo-nextest.bytes")
            .context("cargo-mutants request has no nextest length")?,
        "nextest length",
    )?;
    let nextest_path = fields
        .remove("tool.cargo-nextest.path")
        .context("cargo-mutants request has no nextest path")?
        .to_string();
    let nextest_parent = fields
        .remove("environment.PATH.nextest-first")
        .context("cargo-mutants request has no nextest PATH authority")?;
    ensure!(
        Path::new(&cargo_mutants_path).is_absolute()
            && Path::new(&nextest_path).is_absolute()
            && Path::new(&nextest_path).parent() == Some(Path::new(nextest_parent)),
        "cargo-mutants request tool paths are malformed"
    );
    ensure!(
        fields.is_empty(),
        "cargo-mutants request has unknown fields"
    );
    Ok(ToolIdentity {
        cargo_mutants_path,
        cargo_mutants_bytes,
        cargo_mutants_sha256: CARGO_MUTANTS_EXECUTABLE_SHA256.to_string(),
        nextest_path,
        nextest_bytes,
    })
}

fn discover_tools(paths: &Paths) -> Result<ToolIdentity> {
    let grader = resolve_path(if cfg!(windows) {
        "cargo-mutants.exe"
    } else {
        "cargo-mutants"
    })?;
    let grader_bytes =
        qualification::read_bounded_file(&grader, 512 * 1024 * 1024, "cargo-mutants executable")?;
    ensure!(
        sha256::hex(&sha256::digest(&grader_bytes)) == CARGO_MUTANTS_EXECUTABLE_SHA256,
        "cargo-mutants executable differs from D-342's reviewed build"
    );
    let version = cargo_mutants_version_command(&grader, paths.root());
    let version = crate::bounded_process::output(
        version,
        Duration::from_secs(5),
        "cargo-mutants version probe",
    )?;
    ensure!(
        version.status.success(),
        "cargo-mutants version probe failed"
    );
    let version =
        String::from_utf8(version.stdout).context("cargo-mutants version is not UTF-8")?;
    ensure!(
        version.trim() == format!("cargo-mutants {VERSION}"),
        "cargo-mutants version differs from D-324"
    );
    let nextest = paths
        .target_dir()
        .join("qualification/tools/cargo-nextest/cargo-nextest.exe");
    let nextest_bytes =
        qualification::read_bounded_file(&nextest, 512 * 1024 * 1024, "cargo-nextest executable")?;
    ensure!(
        sha256::hex(&sha256::digest(&nextest_bytes)) == NEXTEST_EXECUTABLE_SHA256,
        "cargo-nextest executable differs from D-324"
    );
    Ok(ToolIdentity {
        cargo_mutants_path: grader.to_string_lossy().into_owned(),
        cargo_mutants_bytes: grader_bytes.len() as u64,
        cargo_mutants_sha256: sha256::hex(&sha256::digest(&grader_bytes)),
        nextest_path: nextest.to_string_lossy().into_owned(),
        nextest_bytes: nextest_bytes.len() as u64,
    })
}

fn cargo_mutants_version_command(grader: &Path, root: &Path) -> Command {
    let mut command = Command::new(grader);
    command.current_dir(root).args(["mutants", "--version"]);
    command
}

fn inventory_command(grader: &Path, root: &Path) -> Command {
    let mut command = Command::new(grader);
    command.current_dir(root).args(INVENTORY_ARGUMENTS);
    command
}

/// The pinned grader's own candidate listing of the checkout being sealed.
fn capture_inventory(tools: &ToolIdentity, root: &Path) -> Result<Vec<u8>> {
    let output = crate::bounded_process::output_with_limit(
        inventory_command(Path::new(&tools.cargo_mutants_path), root),
        INVENTORY_TIMEOUT,
        "independent cargo-mutants inventory",
        usize::try_from(MAX_JSON_BYTES).context("inventory limit exceeds the address space")?,
    )?;
    ensure!(
        output.status.success(),
        "the independent cargo-mutants inventory exited {}",
        output.status
    );
    Ok(output.stdout)
}

fn doctest_command(paths: &Paths) -> Command {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(paths.root())
        .env(
            "CARGO_TARGET_DIR",
            crate::cargo::child_process_path(paths.target_dir()),
        )
        .args(DOCTEST_ARGUMENTS);
    command
}

/// Cargo's `Doc-tests` header goes to stderr and the harness summary to
/// stdout, so the transcript keeps them in that order.
fn capture_doctest(paths: &Paths) -> Result<Vec<u8>> {
    let output = crate::bounded_process::output(
        doctest_command(paths),
        DOCTEST_TIMEOUT,
        "xtask doctest run",
    )?;
    ensure!(
        output.status.success(),
        "the xtask doctest run exited {}",
        output.status
    );
    let mut transcript = output.stderr;
    transcript.extend_from_slice(&output.stdout);
    Ok(transcript)
}

fn output_directory(grade: &Path) -> PathBuf {
    grade.join(CARGO_MUTANTS_OUTPUT)
}

fn require_outside_records(grade: &Path, records: &Path) -> Result<()> {
    ensure!(
        !grade.starts_with(records),
        "the cargo-mutants grade directory {} lies inside the qualification records {}, \
         which sealing replaces",
        grade.display(),
        records.display()
    );
    Ok(())
}

fn reject_byte_order_mark(bytes: &[u8], label: &str) -> Result<()> {
    ensure!(
        !bytes.starts_with(&[0xFF, 0xFE]) && !bytes.starts_with(&[0xFE, 0xFF]),
        "{label} is UTF-16; save it as UTF-8 without a byte-order mark"
    );
    ensure!(
        !bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
        "{label} begins with a UTF-8 byte-order mark; save it without one"
    );
    Ok(())
}

fn resolve_path(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH is unavailable")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return std::fs::canonicalize(&candidate)
                .with_context(|| format!("canonicalizing {}", candidate.display()));
        }
    }
    bail!("{name} is unavailable on PATH")
}

fn canonical_u64(value: &str, label: &str) -> Result<u64> {
    let parsed = value
        .parse::<u64>()
        .with_context(|| format!("{label} is not a number"))?;
    ensure!(
        parsed != 0 && parsed.to_string() == value,
        "{label} is not canonical"
    );
    Ok(parsed)
}

fn verify_doctest(bytes: &[u8]) -> Result<()> {
    let transcript = std::str::from_utf8(bytes).context("doctest transcript is not UTF-8")?;
    let packages: Vec<_> = transcript
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Doc-tests "))
        .collect();
    ensure!(
        packages == ["xtask"],
        "doctest output names {packages:?} rather than exactly the xtask package"
    );
    let counts: Vec<_> = transcript
        .lines()
        .filter_map(|line| line.trim().strip_prefix("running "))
        .filter_map(|rest| rest.strip_suffix(" tests"))
        .collect();
    ensure!(
        !counts.is_empty() && counts.iter().all(|count| *count == "0"),
        "doctest output does not report only zero-test targets"
    );
    let results: Vec<_> = transcript
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("test result:"))
        .collect();
    ensure!(
        results.len() == counts.len()
            && results.iter().all(|line| {
                line.starts_with(
                    "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;",
                )
            }),
        "doctest output has no exact successful zero-test terminal summary"
    );
    ensure!(
        !transcript.lines().any(|line| {
            let line = line.trim();
            line.starts_with("error:") || line.contains("FAILED")
        }),
        "doctest output reports a failure"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_cargo_mutants_probe_names_its_subcommand() {
        let command = cargo_mutants_version_command(Path::new("cargo-mutants"), Path::new("."));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["mutants", "--version"]
        );
    }

    fn candidate(name: &str) -> String {
        format!(
            r#"{{"diff":"d","file":"xtask/src/lib.rs","function":{{}},"genre":"FnValue","name":"{name}","package":"xtask","replacement":"false","span":{{}}}}"#
        )
    }

    fn phases(test_status: &str) -> String {
        format!(
            r#"[{{"phase":"Build","duration":1,"process_status":"Success","argv":["cargo","nextest","run","--no-run","--verbose","--package=xtask@0.1.0"]}},{{"phase":"Test","duration":2,"process_status":{test_status},"argv":["cargo","nextest","run","--verbose","--package=xtask@0.1.0"]}}]"#
        )
    }

    fn outcomes(summary: &str, end: &str) -> Vec<u8> {
        let baseline = phases("\"Success\"");
        let mutant = match summary {
            "CaughtMutant" | "Failure" => phases(r#"{"Failure":100}"#),
            "MissedMutant" => phases("\"Success\""),
            _ => phases("\"Success\""),
        };
        format!(r#"{{"outcomes":[{{"scenario":"Baseline","summary":"Success","log_path":"log/baseline.log","diff_path":null,"phase_results":{baseline}}},{{"scenario":{{"Mutant":{{"name":"m","package":"xtask","file":"xtask/src/lib.rs","function":{{}},"span":{{}},"replacement":"false","genre":"FnValue"}}}},"summary":"{summary}","log_path":"log/m.log","diff_path":"diff/m.diff","phase_results":{mutant}}}],"total_mutants":1,"missed":0,"caught":1,"timeout":0,"unviable":0,"success":0,"start_time":"2026-09-27T09:03:24.9388039Z","end_time":{end},"cargo_mutants_version":"27.1.0"}}"#).into_bytes()
    }

    #[test]
    fn complete_grade_has_one_baseline_and_an_exact_candidate_bijection() {
        let candidates =
            parse_candidates(format!("[{}]", candidate("m")).as_bytes(), "fixture").unwrap();
        let (grade, noncaught) = parse_outcomes(
            &outcomes("CaughtMutant", "\"2026-09-27T09:06:03.3224359Z\""),
            &candidates,
        )
        .unwrap();
        assert_eq!(grade.total, 1);
        assert_eq!(grade.caught, 1);
        assert!(noncaught.is_empty());
        let duplicate = format!("[{},{}]", candidate("m"), candidate("m"));
        assert!(parse_candidates(duplicate.as_bytes(), "duplicate fixture").is_err());
    }

    #[test]
    fn interrupted_and_unclassified_runs_are_not_qualifications() {
        let candidates = BTreeMap::from([("m".to_string(), "d".to_string())]);
        assert!(parse_outcomes(&outcomes("CaughtMutant", "null"), &candidates).is_err());
        assert!(
            parse_outcomes(
                &outcomes("Failure", "\"2026-09-27T09:06:03.3224359Z\""),
                &candidates
            )
            .is_err()
        );
    }

    #[test]
    fn every_non_caught_outcome_needs_one_equivalence_adjudication() {
        let expected = BTreeMap::from([("m".to_string(), "MissedMutant".to_string())]);
        let good = br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[{"name":"m","summary":"MissedMutant","disposition":"equivalent","rationale":"the replacement is observationally identical"}]}"#;
        assert_eq!(verify_adjudications(good, &expected).unwrap(), 1);
        let missing = br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[]}"#;
        assert!(verify_adjudications(missing, &expected).is_err());
    }

    #[test]
    fn request_binds_source_command_concurrency_and_runner_provenance() {
        let source = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let fixture_root = std::env::current_dir().unwrap();
        let tools = ToolIdentity {
            cargo_mutants_path: fixture_root
                .join("cargo-mutants.exe")
                .to_string_lossy()
                .into_owned(),
            cargo_mutants_bytes: 12,
            cargo_mutants_sha256: CARGO_MUTANTS_EXECUTABLE_SHA256.to_string(),
            nextest_path: fixture_root
                .join("cargo-nextest.exe")
                .to_string_lossy()
                .into_owned(),
            nextest_bytes: 34,
        };
        let request = render_request(&source, &tools).unwrap();
        assert!(
            String::from_utf8_lossy(&request).contains(
                "command=cargo mutants -p xtask --output <source-bound-directory> --jobs 1"
            )
        );
        assert_eq!(verify_request(&request, &source).unwrap(), tools);
        let other_grader = ToolIdentity {
            cargo_mutants_sha256: "c".repeat(64),
            ..tools.clone()
        };
        assert!(verify_request(&render_request(&source, &other_grader).unwrap(), &source).is_err());
        let weakened = String::from_utf8(request)
            .unwrap()
            .replace("--jobs 1", "--jobs 2");
        let (body, _) = weakened.rsplit_once("checksum.sha256=").unwrap();
        let weakened = format!(
            "{body}checksum.sha256={}\n",
            sha256::hex(&sha256::digest(body.as_bytes()))
        );
        assert!(verify_request(weakened.as_bytes(), &source).is_err());
    }

    #[test]
    fn independent_inventory_must_match_run_identities_and_diffs() {
        let run = parse_candidates(format!("[{}]", candidate("m")).as_bytes(), "run").unwrap();
        let different = candidate("m").replace(r#""diff":"d""#, r#""diff":"different""#);
        let independent =
            parse_candidates(format!("[{different}]").as_bytes(), "inventory").unwrap();
        assert!(require_same_inventory(&run, &independent).is_err());
    }

    #[test]
    fn phase_summary_requires_exact_order_status_and_nextest_argv() {
        let valid = phases(r#"{"Failure":100}"#);
        require_nextest_phases(
            &parse_json(valid.as_bytes(), "phases").unwrap(),
            "CaughtMutant",
        )
        .unwrap();
        let forged = phases("\"Success\"");
        assert!(
            require_nextest_phases(
                &parse_json(forged.as_bytes(), "phases").unwrap(),
                "CaughtMutant"
            )
            .is_err()
        );
        let wrong_order = valid.replacen(r#""phase":"Build""#, r#""phase":"Test""#, 1);
        assert!(
            require_nextest_phases(
                &parse_json(wrong_order.as_bytes(), "phases").unwrap(),
                "CaughtMutant"
            )
            .is_err()
        );
        let wrong_argv = valid.replace("--package=xtask@0.1.0", "--package=other@0.1.0");
        assert!(
            require_nextest_phases(
                &parse_json(wrong_argv.as_bytes(), "phases").unwrap(),
                "CaughtMutant"
            )
            .is_err()
        );
    }

    #[test]
    fn doctest_transcript_requires_only_successful_zero_test_summaries() {
        let good = "   Doc-tests xtask\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
        verify_doctest(good.as_bytes()).unwrap();
        let forged = format!("{good}error: later failure\n");
        assert!(verify_doctest(forged.as_bytes()).is_err());
        assert!(verify_doctest(&[0xff]).is_err());
    }

    #[test]
    fn doctest_transcript_belongs_to_exactly_the_xtask_package() {
        let good = "   Doc-tests xtask\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
        let headless = good.replacen("   Doc-tests xtask\n", "", 1);
        assert!(verify_doctest(headless.as_bytes()).is_err());
        let foreign = good.replacen("Doc-tests xtask", "Doc-tests fat32", 1);
        assert!(verify_doctest(foreign.as_bytes()).is_err());
        let doubled = format!("   Doc-tests other\n{good}");
        assert!(verify_doctest(doubled.as_bytes()).is_err());
    }

    #[test]
    fn the_published_output_directory_is_the_one_the_sealer_reads() {
        let arguments = COMMAND.split_whitespace().collect::<Vec<_>>();
        let output = arguments
            .windows(2)
            .find(|pair| pair[0] == "--output")
            .map(|pair| pair[1]);
        // cargo-mutants 27.1.0 creates `mutants.out` inside its `--output`
        // argument (its `output.rs`), and the sealer reads that child.
        assert_eq!(output, Some("<source-bound-directory>"));
        assert_eq!(
            output_directory(Path::new("grade")),
            Path::new("grade").join("mutants.out")
        );
    }

    fn baseline_object() -> String {
        format!(
            r#"{{"scenario":"Baseline","summary":"Success","log_path":"log/baseline.log","diff_path":null,"phase_results":{}}}"#,
            phases("\"Success\"")
        )
    }

    #[test]
    fn a_grade_ends_after_it_starts_and_recounts_every_outcome() {
        let candidates = BTreeMap::from([("m".to_string(), "d".to_string())]);
        let valid = String::from_utf8(outcomes("CaughtMutant", "\"2026-09-27T09:06:03.3224359Z\""))
            .unwrap();
        parse_outcomes(valid.as_bytes(), &candidates).unwrap();
        let instantaneous = valid.replace(
            "\"end_time\":\"2026-09-27T09:06:03.3224359Z\"",
            "\"end_time\":\"2026-09-27T09:03:24.9388039Z\"",
        );
        assert!(parse_outcomes(instantaneous.as_bytes(), &candidates).is_err());
        let unclassified = valid.replace("\"success\":0", "\"success\":1");
        assert!(parse_outcomes(unclassified.as_bytes(), &candidates).is_err());
        let miscounted = valid.replace("\"caught\":1", "\"caught\":2");
        assert!(parse_outcomes(miscounted.as_bytes(), &candidates).is_err());
    }

    #[test]
    fn a_grade_has_one_baseline_and_an_outcome_for_every_candidate() {
        let candidates = BTreeMap::from([("m".to_string(), "d".to_string())]);
        let valid = String::from_utf8(outcomes("CaughtMutant", "\"2026-09-27T09:06:03.3224359Z\""))
            .unwrap();
        let baseline = baseline_object();
        assert!(valid.contains(&baseline));
        let doubled = valid.replacen(&baseline, &format!("{baseline},{baseline}"), 1);
        assert!(parse_outcomes(doubled.as_bytes(), &candidates).is_err());
        let absent = valid.replacen(&format!("{baseline},"), "", 1);
        assert!(parse_outcomes(absent.as_bytes(), &candidates).is_err());
        // Counters agree with the larger inventory, so only the missing
        // outcome can refuse it.
        let two = BTreeMap::from([
            ("m".to_string(), "d".to_string()),
            ("n".to_string(), "e".to_string()),
        ]);
        let partial = valid.replace("\"total_mutants\":1", "\"total_mutants\":2");
        assert!(parse_outcomes(partial.as_bytes(), &two).is_err());
    }

    #[test]
    fn adjudications_name_exactly_the_noncaught_mutants_with_allowed_reasons() {
        let expected = BTreeMap::from([("m".to_string(), "MissedMutant".to_string())]);
        let row = |name: &str, disposition: &str, rationale: &str| {
            format!(
                r#"{{"name":"{name}","summary":"MissedMutant","disposition":"{disposition}","rationale":"{rationale}"}}"#
            )
        };
        let document = |rows: &[String]| {
            format!(
                r#"{{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[{}]}}"#,
                rows.join(",")
            )
        };
        let good = row("m", "equivalent", "observationally identical");
        assert_eq!(
            verify_adjudications(document(std::slice::from_ref(&good)).as_bytes(), &expected)
                .unwrap(),
            1
        );
        let wrong_reason = row("m", "compiler-rejected", "observationally identical");
        assert!(verify_adjudications(document(&[wrong_reason]).as_bytes(), &expected).is_err());
        let silent = row("m", "equivalent", "  ");
        assert!(verify_adjudications(document(&[silent]).as_bytes(), &expected).is_err());
        let duplicated = document(&[good.clone(), good.clone()]);
        assert!(verify_adjudications(duplicated.as_bytes(), &expected).is_err());
        let extra = row("caught", "equivalent", "observationally identical");
        assert!(verify_adjudications(document(&[good, extra]).as_bytes(), &expected).is_err());
    }

    #[test]
    fn sealed_commands_are_exactly_the_documented_ones() {
        let paths = Paths::new(PathBuf::from("fixture-root"), None);
        assert_eq!(
            format!("cargo {}", INVENTORY_ARGUMENTS.join(" ")),
            INVENTORY_COMMAND
        );
        assert_eq!(
            format!("cargo {}", DOCTEST_ARGUMENTS.join(" ")),
            DOCTEST_COMMAND
        );
        assert_eq!(
            inventory_command(Path::new("grader"), Path::new("fixture-root"))
                .get_args()
                .collect::<Vec<_>>(),
            INVENTORY_ARGUMENTS
        );
        assert_eq!(
            doctest_command(&paths).get_args().collect::<Vec<_>>(),
            DOCTEST_ARGUMENTS
        );
    }

    #[test]
    fn a_grade_directory_inside_the_records_cannot_be_sealed() {
        let records = Path::new("root/target/qualification");
        assert!(
            require_outside_records(&records.join("cargo-mutants-grade/run"), records).is_err()
        );
        assert!(require_outside_records(records, records).is_err());
        require_outside_records(Path::new("root/grades/run"), records).unwrap();
    }

    #[test]
    fn operator_json_is_utf8_without_a_byte_order_mark() {
        for (bytes, expected) in [
            (&[0xFF, 0xFE, b'{'][..], "UTF-16"),
            (&[0xFE, 0xFF, 0, b'{'][..], "UTF-16"),
            (&[0xEF, 0xBB, 0xBF, b'{'][..], "byte-order mark"),
        ] {
            let error = reject_byte_order_mark(bytes, "fixture").unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
        }
        reject_byte_order_mark(b"{}", "fixture").unwrap();
    }
}
