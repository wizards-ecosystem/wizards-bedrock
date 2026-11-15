use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::paths::Paths;

const MODELS: &str = "models";
const CLEAN: &str = "Model checking completed. No error has been found.";

// Each configuration with the invariant TLC must report violated, or none for a correct model.
const RUNS: &[(&str, &str, Option<&str>)] = &[
    ("Ring.tla", "Ring.cfg", None),
    ("RingMemory.tla", "RingMemory.cfg", None),
    (
        "RingMemory.tla",
        "RingMemory.store-relaxed.cfg",
        Some("NoStalePayload"),
    ),
    (
        "RingMemory.tla",
        "RingMemory.load-relaxed.cfg",
        Some("NoStalePayload"),
    ),
    (
        "RingMemory.tla",
        "RingMemory.no-reservation.cfg",
        Some("ReservationsAreLive"),
    ),
    (
        "RingMemory.tla",
        "RingMemory.duplicate-terminal.cfg",
        Some("OneCompletionPerOperation"),
    ),
];

pub(crate) fn verify(paths: &Paths) -> Result<()> {
    crate::vendor::verify_listing(&paths.tla_checksums())?;
    for &(model, config, violated) in RUNS {
        println!("[xtask] model-check: TLC over {MODELS}/{model} with {config}");
        let scratch = paths.target_dir().join("model-check").join(config);
        if scratch.exists() {
            std::fs::remove_dir_all(&scratch)
                .with_context(|| format!("resetting {}", scratch.display()))?;
        }
        std::fs::create_dir_all(&scratch)
            .with_context(|| format!("creating {}", scratch.display()))?;
        let output = Command::new("java")
            .current_dir(paths.root().join(MODELS))
            // TLC's default collector spends most of a short run in garbage collection.
            .arg("-XX:+UseParallelGC")
            .arg("-cp")
            .arg(paths.tla_jar())
            .args(["tlc2.TLC", "-cleanup", "-noGenerateSpecTE", "-metadir"])
            .arg(&scratch)
            .args(["-config", config, model])
            .output()
            .context("could not run java; put the JVM that runs TLC on PATH")?;
        let report = String::from_utf8_lossy(&output.stdout);
        let expected = match violated {
            None => output.status.success() && report.contains(CLEAN),
            Some(invariant) => {
                !output.status.success()
                    && report.contains(&format!("Error: Invariant {invariant} is violated."))
            }
        };
        if !expected {
            print!("{report}");
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
        }
        ensure!(
            expected,
            "TLC did not classify {model} with {config} as expected"
        );
    }
    println!("[xtask] model-check passed with every deliberate mutation caught");
    Ok(())
}
