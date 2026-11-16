use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::paths::{self, Paths};

const PIN: &str = "tools/verus/VERSION";

pub(crate) fn verify(paths: &Paths) -> Result<()> {
    let pin = std::fs::read_to_string(paths.root().join(PIN))
        .with_context(|| format!("reading {PIN}"))?;
    let pinned = |key: &str| {
        pin.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(' '))
            .with_context(|| format!("{PIN} has no {key} line"))
    };
    let (verus, vstd) = (pinned("verus")?, pinned("vstd")?);

    let lock =
        std::fs::read_to_string(paths.root().join("Cargo.lock")).context("reading Cargo.lock")?;
    ensure!(
        lock.contains(&format!("name = \"vstd\"\nversion = \"{vstd}\"\n")),
        "Cargo.lock does not lock vstd {vstd}, which {PIN} pins"
    );
    let output = Command::new("verus")
        .arg("--version")
        .output()
        .context("could not run verus; put the pinned Verus build on PATH")?;
    let report = String::from_utf8_lossy(&output.stdout);
    let reported = report
        .lines()
        .find_map(|line| line.trim().strip_prefix("Version: "));
    ensure!(
        reported == Some(verus),
        "verus reports version {reported:?}, but {PIN} pins {verus}"
    );

    println!("[xtask] verify: Verus {verus} over every crate with verify = true");
    let mut args = vec!["verus", "verify", "--locked", "--workspace"];
    for name in paths::TARGET_PACKAGES {
        args.extend(["--exclude", name]);
    }
    // Roots only: vstd is checked with `--is-vstd`, which `--no-cheating` refuses.
    args.extend([
        "--fwd-verus-args-to",
        "roots",
        "--",
        "--no-cheating",
        "--no-external-by-default",
    ]);
    let status = Command::new(env!("CARGO"))
        .current_dir(paths.root())
        .env("CARGO_TARGET_DIR", paths.target_dir())
        .args(&args)
        .status()
        .context("could not run cargo verus; put cargo-verus on PATH")?;
    ensure!(
        status.success(),
        "verification failed ({status}); rerun one crate with \
         `cargo verus verify -p CRATE -- --no-external-by-default --expand-errors`"
    );
    println!("[xtask] verify passed");
    Ok(())
}
