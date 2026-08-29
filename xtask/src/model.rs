//! Running TLC over the queue protocol's model.
//!
//! `models/Ring.tla` states D-79's admission rule and the four properties it
//! exists to guarantee; TLC enumerates every reachable state of four actors —
//! two of them a userspace given no fairness at all — and reports whether any
//! of them drops a completion (D-112).
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

/// The model and its configuration, relative to the workspace root.
const MODEL_DIR: &str = "models";
const MODEL: &str = "Ring.tla";
const CONFIG: &str = "Ring.cfg";

/// What TLC prints when every invariant held in every reachable state.
const CLEAN: &str = "Model checking completed. No error has been found.";

/// Check the model, and require TLC to say it is clean.
///
/// # Errors
///
/// If the jar is missing or its digest has changed, if `java` cannot be run,
/// or if TLC reports anything other than a clean run — the last including an
/// invariant violation, whose counterexample TLC has already printed.
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
    println!("[xtask] model-check: TLC over {MODEL_DIR}/{MODEL}");
    let output = Command::new(&java.path)
        .current_dir(&models)
        // TLC's default collector spends most of a short run in GC.
        .arg("-XX:+UseParallelGC")
        .arg("-cp")
        .arg(&jar)
        .arg("tlc2.TLC")
        .arg("-config")
        .arg(CONFIG)
        .arg(MODEL)
        .output()
        .with_context(|| format!("could not run the resolved JVM at {}", java.path.display()))?;

    let report = String::from_utf8_lossy(&output.stdout);
    print!("{report}");
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }

    // TLC's exit code is not enough on its own: it reports success for runs
    // that found nothing to check. The sentence is the verdict.
    if !report.contains(CLEAN) {
        bail!("TLC did not report a clean run of {MODEL}");
    }
    println!("[xtask] model-check passed");
    Ok(())
}

/// Require the jar to be present and to be the one that was pinned.
fn verify_jar(jar: &Path) -> Result<()> {
    let bytes = std::fs::read(jar).with_context(|| {
        format!(
            "{} is missing; see models/README.md for where it comes from",
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
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let spec = std::fs::read_to_string(root.join("models/Ring.tla")).expect("Ring.tla");
        let config = std::fs::read_to_string(root.join("models/Ring.cfg")).expect("Ring.cfg");

        // A definition is `Name == ...` at the left margin.
        let defined: Vec<&str> = spec
            .lines()
            .filter(|line| !line.starts_with("\\"))
            .filter_map(|line| line.split_once(" ==").map(|(name, _)| name))
            .filter(|name| !name.is_empty() && !name.contains(' '))
            .collect();

        // The block runs to the first blank line; the comment and
        // `CHECK_DEADLOCK` that follow it are not invariants. Collecting names
        // rather than searching the raw tail is what lets the config be read
        // in both directions.
        let listed: Vec<&str> = config
            .split_once("INVARIANTS")
            .expect("the config names an INVARIANTS block")
            .1
            .lines()
            .skip(1)
            .map(str::trim)
            .take_while(|line| !line.is_empty())
            .collect();

        // The specification's own machinery: the three names TLA+ convention
        // fixes, plus every action `Next` disjoins. Deriving the actions from
        // `Next` rather than listing them is what turns the rest of this test
        // into a rule — an operator added to the model and forgotten in the
        // config has nowhere left to hide. That is what D-148 asked for, and
        // what naming five invariants in an array only looked like.
        let next = spec
            .lines()
            .find(|line| line.starts_with("Next =="))
            .expect("the model defines Next");
        let mut machinery = vec!["vars", "Init", "Next", "Spec"];
        machinery.extend(
            next.trim_start_matches("Next ==")
                .split("\\/")
                .map(str::trim)
                .filter(|name| !name.is_empty()),
        );

        // Everything else the model defines is a property, and a property TLC
        // is not told to check is a property nobody is checking.
        let invariants: Vec<&str> = defined
            .iter()
            .copied()
            .filter(|name| !machinery.contains(name))
            .collect();

        assert!(
            !invariants.is_empty(),
            "the model defines nothing but machinery, so there is nothing to check"
        );

        for invariant in &invariants {
            assert!(
                listed.contains(invariant),
                "`{invariant}` is defined by the model and not checked by the config"
            );
        }

        for name in &listed {
            assert!(
                defined.contains(name),
                "`{name}` is checked by the config and not defined by the model"
            );
        }
    }

    /// Nothing but the model lives in `models/`, unless it is ignored.
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
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
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
            if name == "Ring.tla" || name == "Ring.cfg" {
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
