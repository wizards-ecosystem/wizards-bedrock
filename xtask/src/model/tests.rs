use std::time::Instant;

use super::*;
use crate::test_support::create_directory_symlink;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root")
        .to_path_buf()
}

#[test]
fn a_missing_model_checker_points_to_its_vendoring_instructions() {
    let missing = root().join("tools/tla/not-the-vendored-model-checker.jar");
    assert!(!missing.exists(), "the missing-jar fixture exists");
    let error = verify_jar(&missing).expect_err("a missing jar must fail");
    assert!(
        format!("{error:#}").contains("tools/tla/README.md"),
        "missing model checker has no usable recovery instruction"
    );
}

#[test]
fn a_redirected_model_scratch_is_refused_without_touching_outside_bytes() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-model-scratch");
    let repository = fixture.join("repository");
    let target = fixture.join("target");
    let state = target.join("model-check");
    let outside = fixture.join("outside");
    std::fs::create_dir_all(&repository).expect("fixture repository");
    std::fs::create_dir_all(&state).expect("model state");
    std::fs::create_dir(&outside).expect("outside directory");
    let sentinel = outside.join("keep");
    std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
    create_directory_symlink(&outside, &state.join(RUNS[0].config));
    let paths = Paths::new(repository, Some(&target));

    assert!(canonical_model_scratch(&state.join(RUNS[0].config)).is_err());
    assert!(prepare_scratch(&paths, &RUNS[0]).is_err());
    assert_eq!(
        std::fs::read(&sentinel).expect("outside sentinel survives"),
        b"outside authority"
    );
    std::fs::remove_dir_all(fixture).expect("remove model scratch fixture");
}

fn listed_invariants(config: &str) -> Vec<&str> {
    config
        .split_once("INVARIANTS")
        .expect("the config names an INVARIANTS block")
        .1
        .lines()
        .skip(1)
        .map(str::trim)
        .take_while(|line| !line.is_empty())
        .collect()
}

fn defined_properties(spec: &str) -> Vec<&str> {
    spec.split_once("Spec ==")
        .expect("the model defines Spec")
        .1
        .lines()
        .filter(|line| !line.starts_with("\\"))
        .filter_map(|line| line.split_once(" ==").map(|(name, _)| name))
        .filter(|name| !name.is_empty() && !name.contains(' '))
        .collect()
}

fn named_set<'spec>(spec: &'spec str, name: &str) -> Vec<&'spec str> {
    let prefix = format!("{name} == {{");
    let line = spec
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("the model defines {name}"));
    line.trim_start_matches(&prefix)
        .trim_end_matches('}')
        .split(',')
        .map(|member| member.trim().trim_matches('"'))
        .collect()
}

/// Every model invariant must appear in its TLC configuration; otherwise
/// TLC can report a clean run without checking it (D-148).
#[test]
fn every_invariant_the_model_defines_is_checked() {
    let root = root();
    for run in RUNS.iter().filter(|run| run.expected == Expected::Clean) {
        let spec = std::fs::read_to_string(root.join(MODEL_DIR).join(run.model))
            .unwrap_or_else(|_| panic!("reading {}", run.model));
        let config = std::fs::read_to_string(root.join(MODEL_DIR).join(run.config))
            .unwrap_or_else(|_| panic!("reading {}", run.config));
        let mut defined = defined_properties(&spec);
        let mut listed = listed_invariants(&config);

        assert!(!defined.is_empty(), "{} defines no properties", run.model);
        defined.sort_unstable();
        listed.sort_unstable();
        assert_eq!(
            listed, defined,
            "{} does not check every property {} defines",
            run.config, run.model
        );
    }
}

/// Every bad configuration changes one rule and names its exact failure.
#[test]
fn every_model_mutation_names_one_exact_invariant() {
    let root = root();
    for run in RUNS {
        let Expected::Violates(expected) = run.expected else {
            continue;
        };
        let config = std::fs::read_to_string(root.join(MODEL_DIR).join(run.config))
            .unwrap_or_else(|_| panic!("reading {}", run.config));
        assert_eq!(listed_invariants(&config), [expected], "{}", run.config);
        assert_eq!(
            config.matches(" = FALSE").count(),
            1,
            "{} is not one isolated model mutation",
            run.config
        );
    }
}

/// Every terminal cause in the model's own domain has a reachable action.
#[test]
fn every_terminal_cause_is_a_reachable_model_action() {
    let spec = std::fs::read_to_string(root().join(MODEL_DIR).join("RingMemory.tla"))
        .expect("reading RingMemory.tla");
    let next = spec
        .split_once("Next ==")
        .expect("the model defines Next")
        .1
        .split_once("Spec ==")
        .expect("Spec follows Next")
        .0;

    for cause in named_set(&spec, "Causes")
        .into_iter()
        .filter(|cause| *cause != "none")
    {
        let mut letters = cause.chars();
        let title = format!(
            "{}{}",
            letters
                .next()
                .expect("a nonempty cause")
                .to_ascii_uppercase(),
            letters.as_str()
        );
        let action = format!("Begin{title}(op)");
        assert!(
            spec.contains(&format!("{action} == BeginSettlement(op, \"{cause}\")")),
            "terminal cause `{cause}` has no distinct settlement action"
        );
        assert!(
            next.contains(&format!("\\/ {action}")),
            "terminal cause `{cause}` is unreachable from Next"
        );
    }
}

/// Only registered model inputs may live in `models/`; ignored TLC scratch
/// files are checked from the filesystem without adding a Git dependency.
#[test]
fn models_holds_the_model_and_nothing_else_unignored() {
    let root = root();
    let ignore = std::fs::read_to_string(root.join(".gitignore")).expect(".gitignore");

    let rules: Vec<&str> = ignore
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("/models/"))
        .map(|rule| rule.trim_end_matches('/'))
        .collect();
    assert!(!rules.is_empty(), "no rule covers models/ at all");

    for entry in
        crate::paths::directory_entries(&root.join("models")).expect("complete models/ listing")
    {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a model path with a textual file name");
        if RUNS
            .iter()
            .any(|run| name == run.model || name == run.config)
        {
            continue;
        }
        assert!(
            rules.iter().any(|rule| glob_lite(rule, name)),
            "`models/{name}` is neither the model nor ignored"
        );
    }
}

/// `*` against a file name, which is all the rules here need.
fn glob_lite(rule: &str, name: &str) -> bool {
    let mut rest = name;
    let mut parts = rule.split('*').peekable();
    let first = parts.next().unwrap_or_default();
    let Some(stripped) = rest.strip_prefix(first) else {
        return false;
    };
    rest = stripped;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        let Some(at) = rest.find(part) else {
            return false;
        };
        rest = &rest[at + part.len()..];
    }
    true
}

#[test]
fn a_tlc_verdict_is_complete_unambiguous_and_exact() {
    let clean = format!(
        "Starting...\n{CLEAN}\n1 states generated.\nFinished in 00s at (2026-08-30 00:00:00)\n"
    );
    let state = "State 1: <Initial predicate>";
    let violated = "Starting...\nError: Invariant ReservationsAreLive is violated.\n\
            Error: The behavior up to this point is:\nState 1: <Initial predicate>\n\
            Finished in 00s at (2026-08-30 00:00:00)\n";

    assert!(require_expected(Expected::Clean, true, &clean, "").is_ok());
    assert!(
        require_expected(
            Expected::Violates("ReservationsAreLive"),
            false,
            violated,
            ""
        )
        .is_ok()
    );
    assert!(require_expected(Expected::Violates("NoStalePayload"), false, violated, "").is_err());
    assert!(require_expected(Expected::Clean, false, &clean, "").is_err());
    assert!(
        require_expected(
            Expected::Violates("ReservationsAreLive"),
            true,
            violated,
            ""
        )
        .is_err()
    );

    let truncated = violated.lines().take(4).collect::<Vec<_>>().join("\n");
    assert!(classify(&truncated, "").is_err());
    let mixed = format!("{clean}{violated}");
    assert!(classify(&mixed, "").is_err());
    let crashed = violated.replace(state, "Exception in thread main java.lang.OutOfMemoryError");
    assert!(classify(&crashed, "").is_err());
    let throwable = violated.replace(
        state,
        "java.lang.AssertionError: forged terminal\n\tat tlc2.TLC.main(TLC.java:1)\nState 1: <Initial predicate>",
    );
    assert!(classify(&throwable, "").is_err());
    let fatal = violated.replace(
        state,
        "Fatal: corrupted worker\nState 1: <Initial predicate>",
    );
    assert!(classify(&fatal, "").is_err());
    let throwable = violated.replace(
        state,
        "java.lang.Throwable: forged\nState 1: <Initial predicate>",
    );
    assert!(classify(&throwable, "").is_err());
    let throwable = violated.replace(
        state,
        "tlc2.tool.EvalException: forged\nState 1: <Initial predicate>",
    );
    assert!(classify(&throwable, "").is_err());
    let fatal = violated.replace(
        state,
        "# A fatal error has been detected by the Java Runtime Environment:\nState 1: <Initial predicate>",
    );
    assert!(classify(&fatal, "").is_err());
    let fatal = violated.replace(
        state,
        "Internal fatal error: forged\nState 1: <Initial predicate>",
    );
    assert!(classify(&fatal, "").is_err());
    let missing_trace = violated.replace(
        "Error: The behavior up to this point is:\nState 1: <Initial predicate>\n",
        "",
    );
    assert!(classify(&missing_trace, "").is_err());
    let delayed_trace = violated.replace(state, "forged preface\nState 1: <Initial predicate>");
    assert!(classify(&delayed_trace, "").is_err());
    assert!(classify(&violated.replace(state, "State forged"), "").is_err());
    assert!(classify(&violated.replace(state, "State 2: <Initial predicate>"), "").is_err());
    assert!(classify(&violated.replace(state, "State 1:"), "").is_err());
    assert!(classify(&violated.replace(state, "State 1: forged"), "").is_err());
    let clean_with_trace = clean.replace(
        "1 states generated.",
        "Error: The behavior up to this point is:\nState 1: <Initial predicate>\n1 states generated.",
    );
    assert!(classify(&clean_with_trace, "").is_err());
    let clean_state = clean.replace(
        "1 states generated.",
        "State 1: <Initial predicate>\n1 states generated.",
    );
    assert!(classify(&clean_state, "").is_err());
    let unrelated = violated.replace("State 1", "Error: disk failed\nState 1");
    assert!(classify(&unrelated, "").is_err());
    assert!(classify(violated, "fatal diagnostic").is_err());
}

#[test]
fn an_unrelated_tlc_error_is_rejected_before_verdict_shape_validation() {
    let transcript = format!(
        "Starting...\n{CLEAN}\nError: disk failed\n1 states generated.\n\
             Finished in 00s at (2026-08-30 00:00:00)\n"
    );

    let error = classify(&transcript, "").unwrap_err();
    assert_eq!(
        error.to_string(),
        "TLC reported an unrelated error beside its verdict"
    );
}

#[test]
fn a_tlc_runtime_failure_is_rejected_before_verdict_shape_validation() {
    let transcript = format!(
        "Starting...\n{CLEAN}\njava.lang.AssertionError: forged\n1 states generated.\n\
             Finished in 00s at (2026-08-30 00:00:00)\n"
    );

    let error = classify(&transcript, "").unwrap_err();
    assert_eq!(error.to_string(), "TLC reported a runtime failure");
}

#[test]
fn qualified_transcripts_bind_every_registered_run_and_stream() {
    let transcripts = fixture_transcripts();
    require_complete_transcripts(&transcripts).unwrap();
    for transcript in &transcripts {
        verify_transcript(transcript.artifact.name, &transcript.bytes).unwrap();
    }

    let mut missing = transcripts.clone();
    missing.pop();
    assert!(require_complete_transcripts(&missing).is_err());

    let mut reordered = transcripts.clone();
    reordered.swap(0, 1);
    assert!(require_complete_transcripts(&reordered).is_err());

    assert!(
        verify_transcript(TRANSCRIPT_ARTIFACTS[1].name, &transcripts[0].bytes).is_err(),
        "one clean run donated another run's identity"
    );

    let mut changed = transcripts[0].bytes.clone();
    let payload = changed
        .windows(2)
        .position(|window| window == b"\n\n")
        .unwrap()
        + 2;
    changed[payload] ^= 1;
    assert!(verify_transcript(TRANSCRIPT_ARTIFACTS[0].name, &changed).is_err());
}

#[test]
fn model_transcript_construction_refuses_capture_beyond_its_ceiling() {
    let error = transcript_with_capture_limit(&RUNS[0], true, b"123", b"45", 4)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("TLC transcript exceeded the bounded child capture limit"),
        "{error}"
    );
}

#[test]
fn model_transcript_verification_refuses_claimed_capture_beyond_its_ceiling() {
    let transcript = fixture_transcripts().remove(0);
    let separator = transcript
        .bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .unwrap();
    let captured = transcript.bytes.len() - separator - 2;
    let error = verify_transcript_with_capture_limit(
        transcript.artifact.name,
        &transcript.bytes,
        captured - 1,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("model transcript payload exceeds the bounded child capture limit"),
        "{error}"
    );
}

#[test]
#[ignore]
fn timeout_child() {
    loop {
        std::thread::park();
    }
}

#[test]
fn a_tlc_process_has_a_direct_child_return_deadline() {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command.args(["--exact", "model::tests::timeout_child", "--ignored"]);
    let started = Instant::now();
    let error = bounded_output(command, Duration::from_millis(250), "model timeout fixture")
        .expect_err("the parked model process must time out");

    assert!(format!("{error:#}").contains("timed out after 250 ms"));
    assert!(started.elapsed() < Duration::from_secs(5));
}
