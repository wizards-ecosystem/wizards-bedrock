use std::collections::BTreeSet;

use anyhow::{Context, Result, bail, ensure};

use crate::paths::Paths;

const INVENTORY: &str = "supply-chain/inventory.txt";
const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";
const LICENSES: &[&str] = &[
    "Apache-2.0",
    "BSD-2-Clause",
    "MIT",
    "MIT AND OFL-1.1",
    "MIT AND Unicode-3.0",
];

#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Package {
    name: String,
    version: String,
    source: Option<String>,
}

pub(crate) fn verify(paths: &Paths) -> Result<()> {
    let lock =
        std::fs::read_to_string(paths.root().join("Cargo.lock")).context("reading Cargo.lock")?;
    let inventory = std::fs::read_to_string(paths.root().join(INVENTORY))
        .with_context(|| format!("reading {INVENTORY}"))?;
    let mut rows = BTreeSet::new();
    for line in inventory.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.splitn(3, ' ').collect();
        let [name, version, license] = fields.as_slice() else {
            bail!("{INVENTORY} row is not `name version license`: {line}");
        };
        ensure!(
            LICENSES.contains(license),
            "{INVENTORY} gives {name} {version} the unreviewed license {license}"
        );
        ensure!(
            rows.insert((*name, *version)),
            "{INVENTORY} lists {name} {version} twice"
        );
    }

    let packages = packages(&lock)?;
    let mut locked = BTreeSet::new();
    for package in &packages {
        let (name, version) = (package.name.as_str(), package.version.as_str());
        match package.source.as_deref() {
            None => continue,
            Some(CRATES_IO) => {}
            Some(other) => bail!("{name} {version} comes from {other}"),
        }
        ensure!(
            rows.contains(&(name, version)),
            "{INVENTORY} has no row for {name} {version}"
        );
        locked.insert((name, version));
    }
    if let Some((name, version)) = rows.difference(&locked).next() {
        bail!("{INVENTORY} lists {name} {version}, which Cargo.lock does not");
    }
    println!(
        "[xtask] supply chain verified: {} crates.io packages, each with a license",
        locked.len()
    );
    Ok(())
}

fn packages(lock: &str) -> Result<Vec<Package>> {
    let mut packages: Vec<Package> = Vec::new();
    for line in lock.lines() {
        if line == "[[package]]" {
            packages.push(Package::default());
            continue;
        }
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        let Some(package) = packages.last_mut() else {
            continue;
        };
        let value = value.trim_matches('"').to_string();
        match key {
            "name" => package.name = value,
            "version" => package.version = value,
            "source" => package.source = Some(value),
            _ => {}
        }
    }
    ensure!(!packages.is_empty(), "Cargo.lock lists no packages");
    Ok(packages)
}
