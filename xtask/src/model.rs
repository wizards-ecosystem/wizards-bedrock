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

    let models = paths.root().join(MODEL_DIR);
    println!("[xtask] model-check: TLC over {MODEL_DIR}/{MODEL}");
    let output = Command::new("java")
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
        .context("could not run java; the model check needs a JVM on PATH")?;

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

        // A definition is `Name == ...` at the left margin. The invariants are
        // the ones whose bodies are a comparison rather than an action.
        let defined: Vec<&str> = spec
            .lines()
            .filter(|line| !line.starts_with("\\"))
            .filter_map(|line| line.split_once(" ==").map(|(name, _)| name))
            .filter(|name| !name.is_empty() && !name.contains(' '))
            .collect();

        let listed = config
            .split_once("INVARIANTS")
            .expect("the config names an INVARIANTS block")
            .1;

        // The five the config lists today, each of which must still be defined
        // by the model and still named by the config.
        for invariant in [
            "NoDroppedCompletion",
            "ReservationsAreOperations",
            "OneCompletionPerOperation",
            "NothingVanishes",
            "TypeOK",
        ] {
            assert!(
                defined.contains(&invariant),
                "`{invariant}` is checked by the config and not defined by the model"
            );
            assert!(
                listed.contains(invariant),
                "`{invariant}` is defined by the model and not checked by the config"
            );
        }

        // And the block is not empty, which is the failure that reads as a
        // pass.
        assert!(
            listed.trim_start().starts_with("NoDropped"),
            "the INVARIANTS block lists nothing; TLC would check nothing and report success"
        );
    }
}
