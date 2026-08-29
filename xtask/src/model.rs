//! Running TLC over the queue protocol's models.
//!
//! `models/Ring.tla` states D-79's anonymous admission rule.
//! `models/RingMemory.tla` carries D-221's weak-memory publication through
//! identified normal, cancelled and retired operations, then requires four
//! deliberate model mutations to violate their exact invariants (D-229).
//!
//! **Not in `ci`, deliberately.** It needs a JVM, which is the one tool this
//! project cannot vendor into a Rust build, and a gate step that fails on a
//! machine without Java is a gate that teaches people to skip it. It is a
//! named task the phase runs and the ADR records instead — the same treatment
//! `vmware` gets, and for the same reason.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expected {
    Clean,
    Violates(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// What TLC prints when every invariant held in every reachable state.
const CLEAN: &str = "Model checking completed. No error has been found.";

/// Check both models and require every deliberate mutation to fail exactly.
///
/// # Errors
///
/// If the jar is missing or its digest has changed, if `java` cannot be run,
/// or if TLC reports anything other than the expected clean or exact-mutant
/// verdict.
pub fn check(paths: &Paths) -> Result<()> {
    let jar = paths.root().join(JAR);
    verify_jar(&jar)?;
    let java = qualification::java(paths.root())?
        .context("Java is unavailable; put the JVM for TLC on PATH")?;

    let models = paths.root().join(MODEL_DIR);
    println!(
        "[xtask] Java identity: {}",
        java.version.replace('\n', " | ")
    );
    for run in RUNS {
        println!(
            "[xtask] model-check: TLC over {MODEL_DIR}/{} with {}",
            run.model, run.config
        );
        let output = Command::new(&java.path)
            .current_dir(&models)
            // TLC's default collector spends most of a short run in GC.
            .arg("-XX:+UseParallelGC")
            .arg("-cp")
            .arg(&jar)
            .arg("tlc2.TLC")
            .arg("-config")
            .arg(run.config)
            .arg(run.model)
            .output()
            .with_context(|| {
                format!("could not run the resolved JVM at {}", java.path.display())
            })?;

        let report = String::from_utf8_lossy(&output.stdout);
        let error = String::from_utf8_lossy(&output.stderr);
        match run.expected {
            Expected::Clean if output.status.success() && report.contains(CLEAN) => {
                print!("{report}");
                if !error.is_empty() {
                    eprint!("{error}");
                }
            }
            Expected::Violates(invariant) => {
                let marker = format!("Invariant {invariant} is violated.");
                if output.status.success() || !report.contains(&marker) {
                    print!("{report}");
                    if !error.is_empty() {
                        eprint!("{error}");
                    }
                    bail!(
                        "{} with {} did not violate `{invariant}` exactly",
                        run.model,
                        run.config
                    );
                }
                println!(
                    "[xtask] model mutation {} violated {invariant} exactly",
                    run.config
                );
            }
            _ => {
                print!("{report}");
                if !error.is_empty() {
                    eprint!("{error}");
                }
                bail!(
                    "TLC did not report a clean run of {} with {}",
                    run.model,
                    run.config
                );
            }
        }
    }
    println!("[xtask] model-check passed with every deliberate mutation caught");
    Ok(())
}

/// Require the jar to be present and to be the one that was pinned.
fn verify_jar(jar: &Path) -> Result<()> {
    let bytes = std::fs::read(jar).with_context(|| {
        format!(
            "{} is missing; see tools/tla/README.md for where it comes from",
            jar.display()
        )
    })?;
    let digest = sha256::hex(&sha256::digest(&bytes));
    if digest != JAR_DIGEST {
        bail!(
            "{} has digest {digest}, not the pinned {JAR_DIGEST}",
            jar.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn named_set<'a>(spec: &'a str, name: &str) -> Vec<&'a str> {
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

    /// Every invariant the model defines is one the config checks.
    ///
    /// TLC checks what `INVARIANTS` names and nothing else. Delete the block
    /// and it still reports "Model checking completed. No error has been
    /// found", because a run with no invariants cannot find one — so
    /// `cargo xtask model-check` would pass with nothing checked, which is the
    /// exact case the config's own comment says it guards (D-148).
    ///
    /// Milliseconds, in `cargo test`, so it runs at every commit while the
    /// model check itself needs a JVM and is deliberately not in `ci`.
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

    /// Nothing but registered model inputs lives in `models/`, unless ignored.
    ///
    /// TLC writes a states directory per run and a *pair* of files beside a
    /// counterexample — `Ring_TTrace_<stamp>.tla` and a `.bin` companion. The
    /// ignore rule covered the spec and not the binary, so five of them were
    /// committed and nothing said so: the trace is TLC's scratch paper, and a
    /// repository that keeps it is claiming it is part of the model.
    ///
    /// Filesystem-based rather than `git ls-files`, because the gate does not
    /// shell out to git anywhere and should not start here.
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

        for entry in std::fs::read_dir(root.join("models")).expect("models/") {
            let name = entry.expect("a directory entry").file_name();
            let name = name.to_str().expect("a name that is text");
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
}
