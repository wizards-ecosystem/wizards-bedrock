//! Source-bound execution and replay checks for the independent `xtask` grade.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};

use crate::cli::MutantsGradeAction;
use crate::json::{
    JsonValue, json_array, json_field, json_object, json_string, json_u64, parse_json,
    require_json_fields,
};
use crate::paths::Paths;
use crate::qualification::{self, ObservedMutantsArtifacts, SourceIdentity};
use crate::sha256;

const VERSION: &str = "27.1.0";
const NEXTEST_VERSION: &str = "0.9.145";
const NEXTEST_COMMIT: &str = "00af4550ec3b3b9f0e574b897b06acb95d325ba2";
const NEXTEST_URL: &str = "https://get.nexte.st/0.9.145/windows";
const NEXTEST_ARCHIVE_SHA256: &str =
    "5bc4b6789103e9834f596eff2b290cfb5b2b69e5413c78bbaa5acc835ac5baa0";
const NEXTEST_EXECUTABLE_SHA256: &str =
    "3fca621fe9e9a5bb7461367ea74fba4c85de824fdb77519b5678fbf91f40c86f";
const COMMAND: &str = "cargo mutants -p xtask --output <xtask-owned-directory> --jobs 1 --jobserver-tasks 1 --test-tool nextest --timeout-multiplier 3 --no-shuffle";
const CAMPAIGN_ARGUMENTS_BEFORE_OUTPUT: [&str; 4] = ["mutants", "-p", "xtask", "--output"];
const CAMPAIGN_ARGUMENTS_AFTER_OUTPUT: [&str; 8] = [
    "--jobs",
    "1",
    "--jobserver-tasks",
    "1",
    "--test-tool",
    "nextest",
    "--timeout-multiplier",
    "3",
];
const CAMPAIGN_FINAL_ARGUMENT: &str = "--no-shuffle";
const CAMPAIGN_TIMEOUT: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const CAMPAIGN_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
const CAMPAIGN_ROOT: &str = "qualification-runs/cargo-mutants";
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
const MAX_CARGO_CONFIG_BYTES: u64 = 64 * 1024;
const ADJUDICATION_POLICY: &str = "xtask/mutants-adjudications.json";
const PINNED_RUST_RELEASE: &str = "1.97.1";
const PRIVATE_CARGO_HOME: &str = "qualification/cargo-home";
const PRIVATE_TEMP_DIRECTORY: &str = "temp";
const PRIVATE_TEMP_REQUEST: &str = "<xtask-owned-directory>/temp";
static NEXT_CAMPAIGN: AtomicU64 = AtomicU64::new(0);

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
struct NonCaught {
    summary: String,
    diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutableIdentity {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolIdentity {
    cargo_mutants_path: String,
    cargo_mutants_bytes: u64,
    cargo_mutants_sha256: String,
    nextest_path: String,
    nextest_bytes: u64,
    cargo: ExecutableIdentity,
    rustc: ExecutableIdentity,
    rustdoc: ExecutableIdentity,
    host_linker: ExecutableIdentity,
    dlltool: ExecutableIdentity,
    host_target: String,
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
                "xtask-job-v1".to_string(),
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
        MutantsGradeAction::Run => run_campaign(paths),
    }
}

fn run_campaign(paths: &Paths) -> Result<()> {
    let attempt = qualification::begin_mutants_grade(paths)?;
    let (campaign_parent, root) = reserve_campaign_directory(paths, attempt.source())?;
    let private_temp = reserve_private_temp_directory(paths, &root)?;
    let tools = discover_tools(paths, &private_temp)?;
    let adjudications = read(
        &paths.root().join(ADJUDICATION_POLICY),
        MAX_ADJUDICATION_BYTES,
        "source-controlled cargo-mutants adjudication policy",
    )?;
    reject_byte_order_mark(&adjudications, "cargo-mutants adjudication policy")?;
    let request = render_request(attempt.source(), &tools, &adjudications)?;
    attempt.write_mutants_request(&request)?;
    println!(
        "[xtask] running the complete cargo-mutants grade in {}",
        root.display()
    );
    let command = campaign_command(&tools, paths, &root, &private_temp)?;
    let result = execute_campaign(command)?;
    require_campaign_success(result.status, &root)?;
    let output = canonical_directory(&output_directory(&root), "cargo-mutants output directory")?;
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

    let requested_tools = verify_request(&request, attempt.source(), &adjudications)?;
    ensure!(
        requested_tools == tools,
        "the observed campaign differs from its source-bound request"
    );
    // The completeness witness, doctest proof and adjudication policy come
    // from this checkout, never from files beside the observed run.
    let inventory = capture_inventory(&tools, paths, &private_temp)?;
    let doctest = capture_doctest(&tools, paths, &private_temp)?;
    verify_doctest(&doctest)?;
    let candidates = parse_candidates(&mutants, "cargo-mutants run inventory")?;
    let independent = parse_candidates(&inventory, "independent cargo-mutants inventory")?;
    require_same_inventory(&candidates, &independent)?;
    let (mut grade, noncaught) = parse_outcomes(&outcomes, &candidates)?;
    grade.adjudicated = verify_adjudications(&adjudications, &noncaught)?;
    // Inventory is another grader invocation and doctests exercise the pinned
    // Cargo toolchain. Rebind source and both external tools after their final
    // use, immediately before any PASS-producing call.
    require_campaign_identity(
        attempt.source(),
        &qualification::source_identity(paths.root())?,
        &tools,
        &discover_tools(paths, &private_temp)?,
    )?;
    let fields = grade.fields(&tools);
    attempt.pass_observed_mutants(
        &fields,
        ObservedMutantsArtifacts {
            request,
            inventory,
            mutants,
            outcomes,
            adjudications,
            doctest,
        },
    )?;
    remove_campaign_directory(&campaign_parent, &root)?;
    println!("[xtask] trusted cargo-mutants grade recorded; scratch output removed");
    Ok(())
}

pub(crate) fn verify_sealed(
    paths: &Paths,
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
    let source_adjudications = read(
        &paths.root().join(ADJUDICATION_POLICY),
        MAX_ADJUDICATION_BYTES,
        "source-controlled cargo-mutants adjudication policy",
    )?;
    require_source_policy(&adjudications, &source_adjudications)?;
    let tools = verify_request(&request, source, &source_adjudications)?;
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
) -> Result<(Grade, BTreeMap<String, NonCaught>)> {
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
                        noncaught.insert(
                            name.to_string(),
                            NonCaught {
                                summary: summary.to_string(),
                                diff: candidates[name].clone(),
                            },
                        );
                    }
                    "Timeout" => {
                        timeout += 1;
                        noncaught.insert(
                            name.to_string(),
                            NonCaught {
                                summary: summary.to_string(),
                                diff: candidates[name].clone(),
                            },
                        );
                    }
                    "Unviable" => {
                        unviable += 1;
                        noncaught.insert(
                            name.to_string(),
                            NonCaught {
                                summary: summary.to_string(),
                                diff: candidates[name].clone(),
                            },
                        );
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

fn verify_adjudications(bytes: &[u8], expected: &BTreeMap<String, NonCaught>) -> Result<u64> {
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
            &["name", "summary", "diff", "disposition", "rationale"],
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
        let diff = json_string(
            json_field(row, "diff", "adjudication")?,
            "adjudication diff",
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
                .insert(
                    name.to_string(),
                    NonCaught {
                        summary: summary.to_string(),
                        diff: diff.to_string(),
                    },
                )
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

fn render_request(
    source: &SourceIdentity,
    tools: &ToolIdentity,
    adjudications: &[u8],
) -> Result<Vec<u8>> {
    let nextest_parent = Path::new(&tools.nextest_path)
        .parent()
        .context("cargo-nextest path has no parent")?;
    let adjudication_digest = sha256::hex(&sha256::digest(adjudications));
    let body = format!(
        "wizard-cargo-mutants-request-v5\nsource.commit={}\nsource.tree={}\ncommand={}\nenvironment.policy=xtask-owned-cargo-rust-temp-v2\nenvironment.CARGO_BUILD_JOBS=1\nenvironment.CARGO_HOME={}\nenvironment.NEXTEST_TEST_THREADS=1\nenvironment.PATH.nextest-first={}\nenvironment.TEMP={}\ncampaign.runner=xtask-bounded-process-windows-job-v1\ninventory.command={}\ninventory.runner=xtask-post-run\ndoctest.command={}\ndoctest.runner=xtask-post-run\nexecution.authority=xtask-job\nadjudication.policy.path={}\nadjudication.policy.sha256={}\ntool.cargo-mutants.bytes={}\ntool.cargo-mutants.path={}\ntool.cargo-mutants.sha256={}\ntool.cargo-mutants.version={}\ntool.cargo-nextest.archive.sha256={}\ntool.cargo-nextest.bytes={}\ntool.cargo-nextest.commit={}\ntool.cargo-nextest.path={}\ntool.cargo-nextest.sha256={}\ntool.cargo-nextest.url={}\ntool.cargo-nextest.version={}\ntool.cargo.bytes={}\ntool.cargo.path={}\ntool.cargo.sha256={}\ntool.rustc.bytes={}\ntool.rustc.path={}\ntool.rustc.sha256={}\ntool.rustc.release={}\ntool.rustdoc.bytes={}\ntool.rustdoc.path={}\ntool.rustdoc.sha256={}\ntool.host-linker.bytes={}\ntool.host-linker.path={}\ntool.host-linker.sha256={}\ntool.dlltool.bytes={}\ntool.dlltool.path={}\ntool.dlltool.sha256={}\ntool.host-target={}\n",
        source.commit,
        source.tree,
        COMMAND,
        PRIVATE_CARGO_HOME,
        nextest_parent.display(),
        PRIVATE_TEMP_REQUEST,
        INVENTORY_COMMAND,
        DOCTEST_COMMAND,
        ADJUDICATION_POLICY,
        adjudication_digest,
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
        tools.cargo.bytes,
        tools.cargo.path,
        tools.cargo.sha256,
        tools.rustc.bytes,
        tools.rustc.path,
        tools.rustc.sha256,
        PINNED_RUST_RELEASE,
        tools.rustdoc.bytes,
        tools.rustdoc.path,
        tools.rustdoc.sha256,
        tools.host_linker.bytes,
        tools.host_linker.path,
        tools.host_linker.sha256,
        tools.dlltool.bytes,
        tools.dlltool.path,
        tools.dlltool.sha256,
        tools.host_target,
    );
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    Ok(format!("{body}checksum.sha256={checksum}\n").into_bytes())
}

fn verify_request(
    bytes: &[u8],
    source: &SourceIdentity,
    adjudications: &[u8],
) -> Result<ToolIdentity> {
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
        lines.next() == Some("wizard-cargo-mutants-request-v5"),
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
        ("environment.policy", "xtask-owned-cargo-rust-temp-v2"),
        ("environment.CARGO_BUILD_JOBS", "1"),
        ("environment.CARGO_HOME", PRIVATE_CARGO_HOME),
        ("environment.NEXTEST_TEST_THREADS", "1"),
        ("environment.TEMP", PRIVATE_TEMP_REQUEST),
        ("campaign.runner", "xtask-bounded-process-windows-job-v1"),
        ("inventory.command", INVENTORY_COMMAND),
        ("inventory.runner", "xtask-post-run"),
        ("doctest.command", DOCTEST_COMMAND),
        ("doctest.runner", "xtask-post-run"),
        ("execution.authority", "xtask-job"),
        ("adjudication.policy.path", ADJUDICATION_POLICY),
        ("tool.cargo-mutants.sha256", CARGO_MUTANTS_EXECUTABLE_SHA256),
        ("tool.cargo-mutants.version", VERSION),
        ("tool.cargo-nextest.archive.sha256", NEXTEST_ARCHIVE_SHA256),
        ("tool.cargo-nextest.version", NEXTEST_VERSION),
        ("tool.cargo-nextest.commit", NEXTEST_COMMIT),
        ("tool.cargo-nextest.url", NEXTEST_URL),
        ("tool.cargo-nextest.sha256", NEXTEST_EXECUTABLE_SHA256),
        ("tool.rustc.release", PINNED_RUST_RELEASE),
    ]);
    for (key, value) in expected {
        ensure!(
            fields.remove(key) == Some(value),
            "cargo-mutants request `{key}` differs from D-324"
        );
    }
    ensure!(
        fields.remove("adjudication.policy.sha256")
            == Some(sha256::hex(&sha256::digest(adjudications)).as_str()),
        "cargo-mutants request adjudication policy differs from the frozen source"
    );
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
    let cargo = request_executable(&mut fields, "tool.cargo", "Cargo")?;
    let rustc = request_executable(&mut fields, "tool.rustc", "rustc")?;
    let rustdoc = request_executable(&mut fields, "tool.rustdoc", "rustdoc")?;
    let host_linker = request_executable(&mut fields, "tool.host-linker", "host linker")?;
    let dlltool = request_executable(&mut fields, "tool.dlltool", "dlltool")?;
    let host_target = fields
        .remove("tool.host-target")
        .context("cargo-mutants request has no host target")?
        .to_string();
    let nextest_parent = fields
        .remove("environment.PATH.nextest-first")
        .context("cargo-mutants request has no nextest PATH authority")?;
    ensure!(
        Path::new(&cargo_mutants_path).is_absolute()
            && Path::new(&nextest_path).is_absolute()
            && Path::new(&nextest_path).parent() == Some(Path::new(nextest_parent))
            && Path::new(&cargo.path).parent() == Path::new(&rustc.path).parent()
            && Path::new(&cargo.path).parent() == Path::new(&rustdoc.path).parent()
            && !host_target.is_empty()
            && host_target
                .bytes()
                .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') }),
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
        cargo,
        rustc,
        rustdoc,
        host_linker,
        dlltool,
        host_target,
    })
}

fn request_executable(
    fields: &mut BTreeMap<&str, &str>,
    prefix: &str,
    label: &str,
) -> Result<ExecutableIdentity> {
    let bytes_key = format!("{prefix}.bytes");
    let path_key = format!("{prefix}.path");
    let sha256_key = format!("{prefix}.sha256");
    let bytes = canonical_u64(
        fields
            .remove(bytes_key.as_str())
            .with_context(|| format!("cargo-mutants request has no {label} length"))?,
        &format!("{label} length"),
    )?;
    let path = fields
        .remove(path_key.as_str())
        .with_context(|| format!("cargo-mutants request has no {label} path"))?
        .to_string();
    let sha256 = fields
        .remove(sha256_key.as_str())
        .with_context(|| format!("cargo-mutants request has no {label} digest"))?
        .to_string();
    ensure!(
        Path::new(&path).is_absolute()
            && sha256.len() == 64
            && sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "cargo-mutants request {label} identity is malformed"
    );
    Ok(ExecutableIdentity {
        path,
        bytes,
        sha256,
    })
}

fn discover_tools(paths: &Paths, private_temp: &Path) -> Result<ToolIdentity> {
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
    let nextest = paths
        .target_dir()
        .join("qualification/tools/cargo-nextest/cargo-nextest.exe");
    let nextest_bytes =
        qualification::read_bounded_file(&nextest, 512 * 1024 * 1024, "cargo-nextest executable")?;
    ensure!(
        sha256::hex(&sha256::digest(&nextest_bytes)) == NEXTEST_EXECUTABLE_SHA256,
        "cargo-nextest executable differs from D-324"
    );
    let cargo_path = std::fs::canonicalize(env!("CARGO"))
        .with_context(|| format!("canonicalizing Cargo executable {}", env!("CARGO")))?;
    let toolchain_bin = cargo_path
        .parent()
        .context("the Cargo executable has no toolchain directory")?;
    let executable_suffix = std::env::consts::EXE_SUFFIX;
    let rustc_path = std::fs::canonicalize(toolchain_bin.join(format!("rustc{executable_suffix}")))
        .context("resolving rustc beside the exact Cargo executable")?;
    let rustdoc_path =
        std::fs::canonicalize(toolchain_bin.join(format!("rustdoc{executable_suffix}")))
            .context("resolving rustdoc beside the exact Cargo executable")?;
    let mut version = Command::new(&rustc_path);
    version.current_dir(paths.root()).arg("-vV");
    remove_ambient_execution_semantics(&mut version, std::env::vars_os().map(|(name, _)| name));
    apply_private_temp_environment(&mut version, private_temp);
    let version = crate::bounded_process::output_with_limit(
        version,
        Duration::from_secs(5),
        "pinned rustc version probe",
        MAX_RECEIPT_BYTES as usize,
    )?;
    ensure!(
        version.status.success(),
        "pinned rustc version probe failed"
    );
    let version =
        std::str::from_utf8(&version.stdout).context("pinned rustc version is not UTF-8")?;
    let releases = version
        .lines()
        .filter_map(|line| line.strip_prefix("release: "))
        .collect::<Vec<_>>();
    let hosts = version
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect::<Vec<_>>();
    let ([release], [host_target]) = (releases.as_slice(), hosts.as_slice()) else {
        bail!("pinned rustc did not report exactly one release and host target");
    };
    ensure!(
        *release == PINNED_RUST_RELEASE,
        "rustc release differs from rust-toolchain.toml"
    );
    ensure!(
        *host_target == "x86_64-pc-windows-gnu",
        "cargo-mutants qualification requires the reviewed Windows GNU host toolchain"
    );
    let mut sysroot = Command::new(&rustc_path);
    sysroot
        .current_dir(paths.root())
        .args(["--print", "sysroot"]);
    remove_ambient_execution_semantics(&mut sysroot, std::env::vars_os().map(|(name, _)| name));
    apply_private_temp_environment(&mut sysroot, private_temp);
    let sysroot = crate::bounded_process::output_with_limit(
        sysroot,
        Duration::from_secs(5),
        "pinned rustc sysroot probe",
        MAX_RECEIPT_BYTES as usize,
    )?;
    ensure!(
        sysroot.status.success(),
        "pinned rustc sysroot probe failed"
    );
    let sysroot = std::str::from_utf8(&sysroot.stdout)
        .context("pinned rustc sysroot is not UTF-8")?
        .trim();
    let sysroot = std::fs::canonicalize(sysroot)
        .with_context(|| format!("canonicalizing rustc sysroot {sysroot}"))?;
    ensure!(
        toolchain_bin.parent() == Some(sysroot.as_path()),
        "Cargo, rustc and the reported sysroot do not share one exact toolchain"
    );
    let linker_path = std::fs::canonicalize(
        sysroot
            .join("lib/rustlib")
            .join(host_target)
            .join("bin/rust-lld.exe"),
    )
    .context("resolving the host linker inside the exact rustc sysroot")?;
    ensure!(
        linker_path.starts_with(&sysroot),
        "the host linker escaped the exact rustc sysroot"
    );
    let dlltool_path = resolve_path("dlltool.exe")?;
    let tools = ToolIdentity {
        cargo_mutants_path: grader.to_string_lossy().into_owned(),
        cargo_mutants_bytes: grader_bytes.len() as u64,
        cargo_mutants_sha256: sha256::hex(&sha256::digest(&grader_bytes)),
        nextest_path: nextest.to_string_lossy().into_owned(),
        nextest_bytes: nextest_bytes.len() as u64,
        cargo: executable_identity(&cargo_path, "Cargo executable")?,
        rustc: executable_identity(&rustc_path, "rustc executable")?,
        rustdoc: executable_identity(&rustdoc_path, "rustdoc executable")?,
        host_linker: executable_identity(&linker_path, "host linker")?,
        dlltool: executable_identity(&dlltool_path, "dlltool executable")?,
        host_target: host_target.to_string(),
    };
    let version = cargo_mutants_version_command(&tools, paths, private_temp)?;
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
    Ok(tools)
}

fn executable_identity(path: &Path, label: &str) -> Result<ExecutableIdentity> {
    let bytes = qualification::read_bounded_file(path, 512 * 1024 * 1024, label)?;
    Ok(ExecutableIdentity {
        path: path.to_string_lossy().into_owned(),
        bytes: bytes.len() as u64,
        sha256: sha256::hex(&sha256::digest(&bytes)),
    })
}

fn cargo_mutants_version_command(
    tools: &ToolIdentity,
    paths: &Paths,
    private_temp: &Path,
) -> Result<Command> {
    let mut command = Command::new(&tools.cargo_mutants_path);
    command
        .current_dir(paths.root())
        .args(["mutants", "--version"]);
    apply_owned_environment(&mut command, tools, paths, private_temp)?;
    Ok(command)
}

fn campaign_command(
    tools: &ToolIdentity,
    paths: &Paths,
    output: &Path,
    private_temp: &Path,
) -> Result<Command> {
    let mut command = Command::new(&tools.cargo_mutants_path);
    command
        .current_dir(paths.root())
        .args(CAMPAIGN_ARGUMENTS_BEFORE_OUTPUT)
        .arg(crate::cargo::child_process_path(output))
        .args(CAMPAIGN_ARGUMENTS_AFTER_OUTPUT)
        .arg(CAMPAIGN_FINAL_ARGUMENT);
    apply_owned_environment(&mut command, tools, paths, private_temp)?;
    Ok(command)
}

fn apply_owned_environment(
    command: &mut Command,
    tools: &ToolIdentity,
    paths: &Paths,
    private_temp: &Path,
) -> Result<()> {
    let nextest_parent = Path::new(&tools.nextest_path)
        .parent()
        .context("cargo-nextest path has no parent")?;
    let cargo_parent = Path::new(&tools.cargo.path)
        .parent()
        .context("Cargo path has no parent")?;
    let inherited_path = std::env::var_os("PATH").context("PATH is unavailable")?;
    let path = std::env::join_paths(
        [nextest_parent.to_path_buf(), cargo_parent.to_path_buf()]
            .into_iter()
            .chain(std::env::split_paths(&inherited_path)),
    )
    .context("constructing the pinned cargo-nextest-first PATH")?;
    let cargo_home = crate::mutation_workspace::prepare_real_directory_tree(
        &paths.target_dir().join(PRIVATE_CARGO_HOME),
        "private cargo-mutants Cargo home",
    )?;
    require_no_cargo_config(&cargo_home, "private cargo-mutants Cargo home")?;
    require_no_generated_cargo_config(private_temp, paths.root())?;
    require_allowlisted_parent_cargo_config(paths.root(), tools)?;
    remove_ambient_execution_semantics(command, std::env::vars_os().map(|(name, _)| name));
    let linker_environment = format!(
        "CARGO_TARGET_{}_LINKER",
        tools
            .host_target
            .to_ascii_uppercase()
            .replace(['-', '.'], "_")
    );
    command
        .env("CARGO", &tools.cargo.path)
        .env("CARGO_HOME", crate::cargo::child_process_path(&cargo_home))
        .env(
            "CARGO_TARGET_DIR",
            crate::cargo::child_process_path(paths.target_dir()),
        )
        .env("CARGO_BUILD_JOBS", "1")
        .env("NEXTEST_TEST_THREADS", "1")
        .env("RUSTC", &tools.rustc.path)
        .env("RUSTDOC", &tools.rustdoc.path)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            format!(
                "-C\u{1f}dlltool={}",
                crate::cargo::child_process_path(Path::new(&tools.dlltool.path)).display()
            ),
        )
        .env(linker_environment, &tools.host_linker.path)
        .env("PATH", path);
    apply_private_temp_environment(command, private_temp);
    Ok(())
}

fn remove_ambient_execution_semantics(
    command: &mut Command,
    names: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for name in names {
        let normalized = name.to_string_lossy().to_ascii_uppercase();
        if normalized.starts_with("CARGO_")
            || normalized.starts_with("RUST")
            || normalized.starts_with("NEXTEST_")
            || normalized.starts_with("MUTANTS_")
            || matches!(
                normalized.as_str(),
                "TARGET"
                    | "CC"
                    | "CXX"
                    | "AR"
                    | "LD"
                    | "CFLAGS"
                    | "CXXFLAGS"
                    | "LDFLAGS"
                    | "TEMP"
                    | "TMP"
                    | "TMPDIR"
            )
        {
            command.env_remove(name);
        }
    }
}

fn apply_private_temp_environment(command: &mut Command, private_temp: &Path) {
    let private_temp = crate::cargo::child_process_path(private_temp);
    for name in ["TEMP", "TMP", "TMPDIR"] {
        command.env(name, &private_temp);
    }
}

fn require_no_cargo_config(directory: &Path, label: &str) -> Result<()> {
    for name in ["config", "config.toml"] {
        ensure!(
            !directory.join(name).exists(),
            "{label} contains caller-controlled Cargo configuration `{name}`"
        );
    }
    Ok(())
}

fn require_no_generated_cargo_config(private_temp: &Path, repository: &Path) -> Result<()> {
    let private_temp = canonical_directory(private_temp, "private cargo-mutants temp root")?;
    let repository = canonical_directory(repository, "cargo-mutants repository root")?;
    ensure!(
        private_temp != repository && private_temp.starts_with(&repository),
        "private cargo-mutants temp root escaped the repository"
    );
    for ancestor in private_temp.ancestors() {
        if ancestor == repository {
            return Ok(());
        }
        require_no_cargo_config(
            &ancestor.join(".cargo"),
            "generated cargo-mutants temp discovery chain",
        )?;
    }
    bail!("private cargo-mutants temp root has no repository ancestor")
}

fn require_allowlisted_parent_cargo_config(root: &Path, tools: &ToolIdentity) -> Result<()> {
    let mut observed = 0usize;
    for ancestor in root.parent().into_iter().flat_map(Path::ancestors) {
        let directory = ancestor.join(".cargo");
        for name in ["config", "config.toml"] {
            let path = directory.join(name);
            if path.exists() {
                observed += 1;
                ensure!(
                    observed == 1,
                    "more than one repository-parent Cargo configuration is active"
                );
                let bytes = qualification::read_bounded_file(
                    &path,
                    MAX_CARGO_CONFIG_BYTES,
                    "repository-parent Cargo configuration",
                )?;
                require_allowlisted_cargo_config_bytes(
                    &bytes,
                    &tools.host_target,
                    &tools.dlltool.path,
                )?;
            }
        }
    }
    Ok(())
}

fn require_allowlisted_cargo_config_bytes(
    bytes: &[u8],
    host_target: &str,
    dlltool: &str,
) -> Result<()> {
    let text =
        std::str::from_utf8(bytes).context("repository-parent Cargo configuration is not UTF-8")?;
    let semantic = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let dlltool = crate::cargo::child_process_path(Path::new(dlltool));
    let escaped_dlltool = dlltool
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let expected =
        format!("[target.{host_target}]\nrustflags = [\"-C\", \"dlltool={escaped_dlltool}\"]");
    ensure!(
        semantic == expected,
        "repository-parent Cargo configuration contains semantics outside the exact pinned dlltool compatibility setting: observed {semantic:?}, expected {expected:?}"
    );
    Ok(())
}

fn execute_campaign(command: Command) -> Result<std::process::Output> {
    crate::bounded_process::output_with_limit(
        command,
        CAMPAIGN_TIMEOUT,
        "complete cargo-mutants campaign",
        CAMPAIGN_CAPTURE_BYTES,
    )
}

fn require_campaign_success(status: ExitStatus, root: &Path) -> Result<()> {
    ensure!(
        status.success(),
        "the complete cargo-mutants campaign exited {status}; scratch output is retained at {}",
        root.display()
    );
    Ok(())
}

fn require_campaign_identity(
    expected_source: &SourceIdentity,
    observed_source: &SourceIdentity,
    expected_tools: &ToolIdentity,
    observed_tools: &ToolIdentity,
) -> Result<()> {
    ensure!(
        expected_source == observed_source,
        "the qualification source changed during the cargo-mutants campaign"
    );
    ensure!(
        expected_tools == observed_tools,
        "cargo-mutants or cargo-nextest changed during the campaign"
    );
    Ok(())
}

fn require_source_policy(sealed: &[u8], source: &[u8]) -> Result<()> {
    ensure!(
        sealed == source,
        "sealed cargo-mutants adjudications differ from the source-controlled policy"
    );
    Ok(())
}

fn reserve_campaign_directory(
    paths: &Paths,
    source: &SourceIdentity,
) -> Result<(PathBuf, PathBuf)> {
    let parent = crate::mutation_workspace::prepare_real_directory_tree(
        &paths.target_dir().join(CAMPAIGN_ROOT),
        "cargo-mutants campaign root",
    )?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..128 {
        let sequence = NEXT_CAMPAIGN.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "{}-{}-{epoch}-{sequence}",
            &source.commit[..12],
            std::process::id()
        );
        let directory = parent.join(name);
        match std::fs::create_dir(&directory) {
            Ok(()) => {
                let directory = canonical_directory(&directory, "cargo-mutants campaign")?;
                ensure!(
                    directory.parent() == Some(parent.as_path()),
                    "reserved cargo-mutants campaign escaped its private root"
                );
                return Ok((parent, directory));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("reserving cargo-mutants campaign {}", directory.display())
                });
            }
        }
    }
    bail!("could not reserve a unique cargo-mutants campaign directory")
}

fn remove_campaign_directory(parent: &Path, directory: &Path) -> Result<()> {
    let parent = canonical_directory(parent, "cargo-mutants campaign root")?;
    let directory = canonical_directory(directory, "completed cargo-mutants campaign")?;
    ensure!(
        directory.parent() == Some(parent.as_path()) && directory.starts_with(&parent),
        "completed cargo-mutants campaign escaped its private root"
    );
    std::fs::remove_dir_all(&directory)
        .with_context(|| format!("removing completed campaign {}", directory.display()))
}

fn reserve_private_temp_directory(paths: &Paths, campaign: &Path) -> Result<PathBuf> {
    let campaign = canonical_directory(campaign, "cargo-mutants campaign")?;
    let directory = campaign.join(PRIVATE_TEMP_DIRECTORY);
    std::fs::create_dir(&directory)
        .with_context(|| format!("reserving private temp root {}", directory.display()))?;
    let directory = canonical_directory(&directory, "private cargo-mutants temp root")?;
    ensure!(
        directory.parent() == Some(campaign.as_path()),
        "private cargo-mutants temp root escaped its campaign"
    );
    require_no_generated_cargo_config(&directory, paths.root())?;
    Ok(directory)
}

fn inventory_command(tools: &ToolIdentity, paths: &Paths, private_temp: &Path) -> Result<Command> {
    let mut command = Command::new(&tools.cargo_mutants_path);
    command.current_dir(paths.root()).args(INVENTORY_ARGUMENTS);
    apply_owned_environment(&mut command, tools, paths, private_temp)?;
    Ok(command)
}

/// The pinned grader's own candidate listing of the checkout being sealed.
fn capture_inventory(tools: &ToolIdentity, paths: &Paths, private_temp: &Path) -> Result<Vec<u8>> {
    let output = crate::bounded_process::output_with_limit(
        inventory_command(tools, paths, private_temp)?,
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

fn doctest_command(tools: &ToolIdentity, paths: &Paths, private_temp: &Path) -> Result<Command> {
    let mut command = Command::new(&tools.cargo.path);
    command.current_dir(paths.root()).args(DOCTEST_ARGUMENTS);
    apply_owned_environment(&mut command, tools, paths, private_temp)?;
    Ok(command)
}

/// Cargo's `Doc-tests` header goes to stderr and the harness summary to
/// stdout, so the transcript keeps them in that order.
fn capture_doctest(tools: &ToolIdentity, paths: &Paths, private_temp: &Path) -> Result<Vec<u8>> {
    let output = crate::bounded_process::output(
        doctest_command(tools, paths, private_temp)?,
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

    fn with_fixture_root<T>(operation: impl FnOnce(&Path) -> T) -> T {
        let root = crate::test_support::reserve_temp_directory("wizard-mutants-grade-fixture");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&root)));
        std::fs::remove_dir_all(&root).expect("removing cargo-mutants test fixture");
        match outcome {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn fixture_executable(root: &Path, name: &str) -> ExecutableIdentity {
        ExecutableIdentity {
            path: root.join(name).to_string_lossy().into_owned(),
            bytes: 1,
            sha256: "a".repeat(64),
        }
    }

    fn fixture_tools(root: &Path) -> ToolIdentity {
        ToolIdentity {
            cargo_mutants_path: root
                .join("cargo-mutants.exe")
                .to_string_lossy()
                .into_owned(),
            cargo_mutants_bytes: 12,
            cargo_mutants_sha256: CARGO_MUTANTS_EXECUTABLE_SHA256.to_string(),
            nextest_path: root
                .join("cargo-nextest.exe")
                .to_string_lossy()
                .into_owned(),
            nextest_bytes: 34,
            cargo: fixture_executable(root, "cargo.exe"),
            rustc: fixture_executable(root, "rustc.exe"),
            rustdoc: fixture_executable(root, "rustdoc.exe"),
            host_linker: fixture_executable(root, "rust-lld.exe"),
            dlltool: fixture_executable(
                resolve_path("dlltool.exe")
                    .unwrap_or_else(|_| root.join("dlltool.exe"))
                    .parent()
                    .unwrap_or(root),
                "dlltool.exe",
            ),
            host_target: "x86_64-pc-windows-gnu".to_string(),
        }
    }

    fn fixture_private_temp(paths: &Paths) -> PathBuf {
        crate::mutation_workspace::prepare_real_directory_tree(
            &paths
                .target_dir()
                .join("qualification/test-cargo-mutants-temp"),
            "test cargo-mutants temp root",
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn failure_status() -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(1)
    }

    #[cfg(windows)]
    fn failure_status() -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(1)
    }

    #[test]
    fn direct_cargo_mutants_probe_names_its_subcommand() {
        with_fixture_root(|root| {
            let paths = Paths::new(root.to_path_buf(), Some(&root.join("target")));
            let tools = fixture_tools(root);
            let private_temp = fixture_private_temp(&paths);
            let command = cargo_mutants_version_command(&tools, &paths, &private_temp).unwrap();
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                ["mutants", "--version"]
            );
        });
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
        let expected = BTreeMap::from([(
            "m".to_string(),
            NonCaught {
                summary: "MissedMutant".to_string(),
                diff: "d".to_string(),
            },
        )]);
        let good = br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[{"name":"m","summary":"MissedMutant","diff":"d","disposition":"equivalent","rationale":"the replacement is observationally identical"}]}"#;
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
        let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let tools = fixture_tools(fixture_root);
        let adjudications = br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[]}"#;
        let request = render_request(&source, &tools, adjudications).unwrap();
        assert!(
            String::from_utf8_lossy(&request).contains(
                "command=cargo mutants -p xtask --output <xtask-owned-directory> --jobs 1"
            )
        );
        assert_eq!(
            verify_request(&request, &source, adjudications).unwrap(),
            tools
        );
        let other_grader = ToolIdentity {
            cargo_mutants_sha256: "c".repeat(64),
            ..tools.clone()
        };
        assert!(
            verify_request(
                &render_request(&source, &other_grader, adjudications).unwrap(),
                &source,
                adjudications,
            )
            .is_err()
        );
        let weakened = String::from_utf8(request.clone())
            .unwrap()
            .replace("--jobs 1", "--jobs 2");
        let (body, _) = weakened.rsplit_once("checksum.sha256=").unwrap();
        let weakened = format!(
            "{body}checksum.sha256={}\n",
            sha256::hex(&sha256::digest(body.as_bytes()))
        );
        assert!(verify_request(weakened.as_bytes(), &source, adjudications).is_err());
        let changed_policy =
            br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[{}]}"#;
        assert!(verify_request(&request, &source, changed_policy).is_err());
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
    fn the_owned_output_directory_is_the_one_the_recorder_reads() {
        let arguments = COMMAND.split_whitespace().collect::<Vec<_>>();
        let output = arguments
            .windows(2)
            .find(|pair| pair[0] == "--output")
            .map(|pair| pair[1]);
        // cargo-mutants 27.1.0 creates `mutants.out` inside its `--output`
        // argument (its `output.rs`), and the recorder reads that child.
        assert_eq!(output, Some("<xtask-owned-directory>"));
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
        let expected = BTreeMap::from([(
            "m".to_string(),
            NonCaught {
                summary: "MissedMutant".to_string(),
                diff: "d".to_string(),
            },
        )]);
        let row = |name: &str, diff: &str, disposition: &str, rationale: &str| {
            format!(
                r#"{{"name":"{name}","summary":"MissedMutant","diff":"{diff}","disposition":"{disposition}","rationale":"{rationale}"}}"#
            )
        };
        let document = |rows: &[String]| {
            format!(
                r#"{{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[{}]}}"#,
                rows.join(",")
            )
        };
        let good = row("m", "d", "equivalent", "observationally identical");
        assert_eq!(
            verify_adjudications(document(std::slice::from_ref(&good)).as_bytes(), &expected)
                .unwrap(),
            1
        );
        let wrong_reason = row("m", "d", "compiler-rejected", "observationally identical");
        assert!(verify_adjudications(document(&[wrong_reason]).as_bytes(), &expected).is_err());
        let wrong_diff = row("m", "different", "equivalent", "observationally identical");
        assert!(verify_adjudications(document(&[wrong_diff]).as_bytes(), &expected).is_err());
        let silent = row("m", "d", "equivalent", "  ");
        assert!(verify_adjudications(document(&[silent]).as_bytes(), &expected).is_err());
        let duplicated = document(&[good.clone(), good.clone()]);
        assert!(verify_adjudications(duplicated.as_bytes(), &expected).is_err());
        let extra = row("caught", "x", "equivalent", "observationally identical");
        assert!(verify_adjudications(document(&[good, extra]).as_bytes(), &expected).is_err());
    }

    #[test]
    fn owned_commands_are_exactly_the_documented_ones() {
        with_fixture_root(|root| {
            let paths = Paths::new(root.to_path_buf(), Some(&root.join("target")));
            let tools = fixture_tools(root);
            let private_temp = fixture_private_temp(&paths);
            assert_eq!(
                format!("cargo {}", INVENTORY_ARGUMENTS.join(" ")),
                INVENTORY_COMMAND
            );
            assert_eq!(
                format!("cargo {}", DOCTEST_ARGUMENTS.join(" ")),
                DOCTEST_COMMAND
            );
            let inventory = inventory_command(&tools, &paths, &private_temp).unwrap();
            assert_eq!(
                inventory.get_program(),
                Path::new(&tools.cargo_mutants_path)
            );
            assert_eq!(
                inventory.get_args().collect::<Vec<_>>(),
                INVENTORY_ARGUMENTS
            );
            let doctest = doctest_command(&tools, &paths, &private_temp).unwrap();
            assert_eq!(doctest.get_program(), Path::new(&tools.cargo.path));
            assert_eq!(doctest.get_args().collect::<Vec<_>>(), DOCTEST_ARGUMENTS);
            let output = root.join("owned-output");
            let campaign = campaign_command(&tools, &paths, &output, &private_temp).unwrap();
            assert_eq!(campaign.get_program(), Path::new(&tools.cargo_mutants_path));
            let output_argument = crate::cargo::child_process_path(&output);
            let expected: Vec<_> = CAMPAIGN_ARGUMENTS_BEFORE_OUTPUT
                .iter()
                .map(std::ffi::OsStr::new)
                .chain(std::iter::once(output_argument.as_os_str()))
                .chain(
                    CAMPAIGN_ARGUMENTS_AFTER_OUTPUT
                        .iter()
                        .map(std::ffi::OsStr::new),
                )
                .chain(std::iter::once(std::ffi::OsStr::new(
                    CAMPAIGN_FINAL_ARGUMENT,
                )))
                .collect();
            assert_eq!(campaign.get_args().collect::<Vec<_>>(), expected);
            for command in [&campaign, &inventory, &doctest] {
                for (name, value) in [
                    ("CARGO_BUILD_JOBS", "1"),
                    ("NEXTEST_TEST_THREADS", "1"),
                    ("RUSTC", tools.rustc.path.as_str()),
                    ("RUSTDOC", tools.rustdoc.path.as_str()),
                ] {
                    assert!(command.get_envs().any(|(observed, observed_value)| {
                        observed == name && observed_value == Some(std::ffi::OsStr::new(value))
                    }));
                }
                let encoded = format!(
                    "-C\u{1f}dlltool={}",
                    crate::cargo::child_process_path(Path::new(&tools.dlltool.path)).display()
                );
                assert!(command.get_envs().any(|(observed, observed_value)| {
                    observed == "CARGO_ENCODED_RUSTFLAGS"
                        && observed_value == Some(std::ffi::OsStr::new(&encoded))
                }));
                let private_temp = crate::cargo::child_process_path(&private_temp);
                for name in ["TEMP", "TMP", "TMPDIR"] {
                    assert!(command.get_envs().any(|(observed, observed_value)| {
                        observed == name && observed_value == Some(private_temp.as_os_str())
                    }));
                }
            }
            let campaign_path = campaign
                .get_envs()
                .find_map(|(name, value)| (name == "PATH").then_some(value).flatten())
                .unwrap();
            assert_eq!(
                std::env::split_paths(campaign_path).next().as_deref(),
                Path::new(&tools.nextest_path).parent()
            );
            let implementation = include_str!("mutants_grade.rs");
            let execute = implementation.find("fn execute_campaign(").unwrap();
            let success = implementation[execute..]
                .find("fn require_campaign_success(")
                .unwrap()
                + execute;
            let body = &implementation[execute..success];
            assert!(body.contains("crate::bounded_process::output_with_limit("));
            assert!(!body.contains("command.output()"));
        });
    }

    #[test]
    fn command_fixtures_do_not_inherit_the_compiling_checkout() {
        with_fixture_root(|root| {
            let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
            assert!(!root.starts_with(checkout));
            let paths = Paths::new(root.to_path_buf(), Some(&root.join("target")));
            let tools = fixture_tools(root);
            let private_temp = fixture_private_temp(&paths);
            cargo_mutants_version_command(&tools, &paths, &private_temp).unwrap();
        });
    }

    #[test]
    fn hostile_compiler_cargo_mutants_and_nextest_environment_is_removed() {
        let hostile = [
            "CARGO_BUILD_RUSTC_WRAPPER",
            "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER",
            "RUSTC",
            "RUSTC_WRAPPER",
            "RUSTFLAGS",
            "RUSTDOCFLAGS",
            "RUSTUP_TOOLCHAIN",
            "TARGET",
            "NEXTEST_RUNNER",
            "NEXTEST_TEST_THREADS",
            "MUTANTS_TEST_TOOL",
            "CC",
            "LDFLAGS",
        ];
        let mut command = Command::new("fixture");
        for name in hostile {
            command.env(name, "attacker-controlled");
        }
        remove_ambient_execution_semantics(
            &mut command,
            hostile.into_iter().map(std::ffi::OsString::from),
        );
        for name in hostile {
            assert!(
                command
                    .get_envs()
                    .any(|(observed, value)| { observed == name && value.is_none() })
            );
        }
    }

    #[test]
    fn parent_cargo_configuration_is_semantically_allowlisted() {
        let dlltool = r"C:\tools\dlltool.exe";
        let allowed = b"# compatibility only\n[target.x86_64-pc-windows-gnu]\nrustflags = [\"-C\", \"dlltool=C:\\\\tools\\\\dlltool.exe\"]\n";
        require_allowlisted_cargo_config_bytes(allowed, "x86_64-pc-windows-gnu", dlltool).unwrap();
        for hostile in [
            b"[build]\nrustc-wrapper = \"attacker\"".as_slice(),
            b"[target.x86_64-pc-windows-gnu]\nrunner = \"attacker\"".as_slice(),
            b"[env]\nRUSTC = { value = \"attacker\", force = true }".as_slice(),
            b"[target.x86_64-pc-windows-gnu]\nrustflags = [\"-C\", \"dlltool=C:\\\\other\\\\dlltool.exe\"]".as_slice(),
        ] {
            assert!(
                require_allowlisted_cargo_config_bytes(
                    hostile,
                    "x86_64-pc-windows-gnu",
                    dlltool,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn owned_temp_root_controls_cargo_configuration_discovery() {
        let fixture = crate::test_support::reserve_temp_directory("wizard-mutants-temp-discovery");
        let repository = fixture.join("repository");
        let target = repository.join("target");
        crate::mutation_workspace::prepare_real_directory_tree(
            &target,
            "temp-discovery fixture target",
        )
        .unwrap();
        let paths = Paths::new(repository.clone(), Some(&target));
        let source = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let (_, campaign) = reserve_campaign_directory(&paths, &source).unwrap();
        let private_temp = reserve_private_temp_directory(&paths, &campaign).unwrap();
        let tools = fixture_tools(&repository);
        let mut command = Command::new("fixture");
        for name in ["TEMP", "TMP", "TMPDIR"] {
            command.env(name, fixture.join("attacker-temp"));
        }
        apply_owned_environment(&mut command, &tools, &paths, &private_temp).unwrap();
        let pinned = crate::cargo::child_process_path(&private_temp);
        for name in ["TEMP", "TMP", "TMPDIR"] {
            assert!(command.get_envs().any(|(observed, value)| {
                observed == name && value == Some(pinned.as_os_str())
            }));
        }

        let hostile = target.join("qualification-runs/.cargo");
        std::fs::create_dir(&hostile).unwrap();
        std::fs::write(
            hostile.join("config.toml"),
            "[target.x86_64-pc-windows-gnu]\nrunner = \"attacker\"\n",
        )
        .unwrap();
        assert!(require_no_generated_cargo_config(&private_temp, &repository).is_err());
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn final_identity_check_follows_every_authoritative_tool_use_and_precedes_pass() {
        let implementation = include_str!("mutants_grade.rs");
        let start = implementation.find("fn run_campaign(").unwrap();
        let end = implementation[start..]
            .find("pub(crate) fn verify_sealed(")
            .unwrap()
            + start;
        let body = &implementation[start..end];
        let inventory = body.find("capture_inventory(").unwrap();
        let doctest = body.find("capture_doctest(").unwrap();
        let identity = body.find("require_campaign_identity(").unwrap();
        let remeasure = body[identity..]
            .find("discover_tools(paths, &private_temp)?")
            .unwrap()
            + identity;
        let pass = body.find("attempt.pass_observed_mutants(").unwrap();
        assert!(inventory < identity);
        assert!(doctest < identity);
        assert!(identity < remeasure);
        assert!(remeasure < pass);
    }

    #[test]
    fn only_a_successful_unchanged_campaign_and_tracked_policy_can_pass() {
        assert!(require_campaign_success(failure_status(), Path::new("scratch")).is_err());
        let source = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let changed_source = SourceIdentity {
            commit: "c".repeat(40),
            tree: source.tree.clone(),
        };
        let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let tools = fixture_tools(fixture_root);
        let changed_tools = ToolIdentity {
            nextest_bytes: 2,
            ..tools.clone()
        };
        assert!(require_campaign_identity(&source, &changed_source, &tools, &tools).is_err());
        assert!(require_campaign_identity(&source, &source, &tools, &changed_tools).is_err());
        require_source_policy(b"tracked", b"tracked").unwrap();
        assert!(require_source_policy(b"supplied", b"tracked").is_err());
    }

    #[test]
    fn campaign_scratch_is_exclusively_reserved_under_target_and_removed() {
        let root = crate::test_support::reserve_temp_directory("wizard-owned-mutants");
        let paths = Paths::new(root.join("repository"), Some(&root.join("target")));
        let source = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let (parent, directory) = reserve_campaign_directory(&paths, &source).unwrap();
        assert_eq!(directory.parent(), Some(parent.as_path()));
        assert!(parent.ends_with(Path::new("qualification-runs/cargo-mutants")));
        let nested = directory.join("nested");
        std::fs::create_dir(&nested).unwrap();
        assert!(remove_campaign_directory(&parent, &nested).is_err());
        std::fs::write(directory.join("owned"), "campaign").unwrap();
        remove_campaign_directory(&parent, &directory).unwrap();
        assert!(!directory.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_controlled_policy_is_utf8_without_a_byte_order_mark() {
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
