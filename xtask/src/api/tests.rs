use super::*;

mod catalog;
mod compile;
mod diagnostics;
mod dispatch;
mod entry;
mod inventory;
mod macros;
mod reviewed;
mod traits;
mod workspace;

const PRODUCTION: &str = concat!(
    include_str!("reviewed.rs"),
    include_str!("catalog.rs"),
    include_str!("inventory.rs"),
    include_str!("entry.rs"),
    include_str!("traits.rs"),
    include_str!("modules.rs"),
    include_str!("diagnostics.rs"),
    include_str!("macros.rs"),
    include_str!("collector.rs"),
    include_str!("dispatch.rs"),
    include_str!("conditions.rs"),
    include_str!("uses.rs"),
    include_str!("workspace.rs"),
    include_str!("compiler.rs"),
    include_str!("compile.rs"),
    include_str!("../api.rs"),
);

#[cfg(test)]
fn fixture_targets(root: &Path) -> Result<Vec<CargoTarget>> {
    let mut paths = Vec::new();
    for directory in COPY_DIRECTORIES {
        let directory = root.join(directory);
        collect_optional_rust_files(&directory, &mut paths)?;
    }
    let mut targets = Vec::new();
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("{} is outside the fixture root", path.display()))?
            .to_path_buf();
        let components = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>();
        let (package, name, kind, executable) = match components.as_slice() {
            ["libs", package, "src", "lib.rs"] => (
                *package,
                package.replace('-', "_"),
                CargoTargetKind::Library,
                false,
            ),
            ["libs", package, "src", "main.rs"] => (
                *package,
                package.replace('-', "_"),
                CargoTargetKind::Bin,
                true,
            ),
            [package @ ("kernel" | "wost" | "xtask"), "src", "lib.rs"] => (
                *package,
                package.replace('-', "_"),
                CargoTargetKind::Library,
                false,
            ),
            [package @ ("kernel" | "wost" | "xtask"), "src", "main.rs"] => (
                *package,
                package.replace('-', "_"),
                CargoTargetKind::Bin,
                true,
            ),
            _ => continue,
        };
        targets.push(CargoTarget {
            package: package.to_string(),
            package_id: package.to_string(),
            name,
            relative,
            kind,
            test: true,
            executable,
        });
    }
    Ok(targets)
}

#[cfg(test)]
fn compiler_input_snapshot(
    sysroot: &Path,
    cargo: &Path,
    compiler: &Path,
    targets: &[String],
) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    let mut snapshot = compiler_runtime_snapshot(sysroot, cargo, compiler, targets)?;
    for (path, digest) in compiler_target_library_snapshot(sysroot, targets)? {
        ensure!(
            snapshot.insert(path.clone(), digest).is_none(),
            "{} occurs twice in the compiler input snapshot",
            path.display()
        );
    }
    Ok(snapshot)
}

#[cfg(test)]
fn cargo_compilation_output(
    bytes: &[u8],
    selected_test: Option<&CargoTarget>,
) -> Result<Vec<MarkerDiagnostic>> {
    let mut evidence = CargoCompilationEvidence::new(selected_test.cloned());
    for line in bytes.split(|byte| *byte == b'\n') {
        evidence.observe(line)?;
    }
    evidence.finish()
}

#[cfg(test)]
fn cargo_test_output(bytes: &[u8], selected: &CargoTarget) -> Result<Vec<MarkerDiagnostic>> {
    cargo_compilation_output(bytes, Some(selected))
}

impl Inventory {
    #[cfg(test)]
    fn discover(root: &Path) -> Result<Self> {
        let targets = fixture_targets(root)?;
        Self::discover_with_targets(root, &targets)
    }

    #[cfg(test)]
    fn discover_with_targets(root: &Path, targets: &[CargoTarget]) -> Result<Self> {
        Self::discover_with_targets_and_entry_authority(root, targets, None)
    }
}

impl TraitImports {
    #[cfg(test)]
    fn collect(items: &[Item]) -> Self {
        Self::collect_in_scope(items, &[])
    }
}

impl Uses {
    #[cfg(test)]
    fn observe(
        &mut self,
        kind: RootKind,
        inventory: &Inventory,
        root: &Path,
        output: &str,
        instrumentation: &Instrumentation,
    ) -> Result<()> {
        self.seed_dispatches(inventory)?;
        for line in output.lines().filter(|line| line.contains(MARKER)) {
            let marker = line
                .find(MARKER)
                .context("an API diagnostic lost its marker")?;
            let marker_digits = line[marker + MARKER.len()..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            let id = <usize as core::str::FromStr>::from_str(&marker_digits)
                .context("an API diagnostic has a malformed marker")?;
            ensure!(
                id < inventory.nodes.len(),
                "an API diagnostic names unknown item {id}"
            );
            ensure!(
                instrumentation.ids.contains(&id),
                "an API diagnostic names item {id}, which was not instrumented in this compiler round"
            );

            let location = &line[..marker];
            let warning = location
                .find(": warning:")
                .context("an API diagnostic is not in short compiler format")?;
            let mut parts = location[..warning].rsplitn(3, ':');
            let column = <usize as core::str::FromStr>::from_str(
                core::iter::Iterator::next(&mut parts)
                    .context("an API diagnostic has no column")?,
            )
            .context("an API diagnostic column is not a number")?;
            let source_line = <usize as core::str::FromStr>::from_str(
                core::iter::Iterator::next(&mut parts).context("an API diagnostic has no line")?,
            )
            .context("an API diagnostic line is not a number")?;
            let reported =
                core::iter::Iterator::next(&mut parts).context("an API diagnostic has no path")?;
            let file = inventory.file_index(root, reported).with_context(|| {
                format!(
                    "an API diagnostic for {} came from unclassified source `{reported}`",
                    inventory.nodes[id].label
                )
            })?;
            self.observe_marker(
                kind,
                inventory,
                MarkerObservation {
                    file,
                    id,
                    source_line,
                    column,
                },
                instrumentation,
                None,
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn validate_dispatches(&self, inventory: &Inventory) -> Result<()> {
        let failures = self.dispatch_failures(inventory);
        ensure!(
            failures.is_empty(),
            "{}",
            failures
                .iter()
                .map(|failure| dispatch_failure(inventory, *failure))
                .collect::<Vec<_>>()
                .join("\n")
        );
        Ok(())
    }

    #[cfg(test)]
    fn dead<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Node> {
        let live = self.live(inventory);
        inventory
            .nodes
            .iter()
            .filter(|node| node.report && !live.contains(&node.id))
            .collect()
    }
}

impl GraphUses {
    #[cfg(test)]
    fn observe(
        &mut self,
        graph: usize,
        kind: RootKind,
        inventory: &Inventory,
        root: &Path,
        output: &str,
        instrumentation: &Instrumentation,
    ) -> Result<()> {
        if self.graphs.len() <= graph {
            self.graphs.resize_with(graph + 1, Uses::default);
        }
        self.graphs[graph].observe(kind, inventory, root, output, instrumentation)
    }
}

fn isolated_test_cargo_command(root: &Path, compiler: &CompilerIdentity) -> Command {
    let cargo = root.join("audit-cargo");
    let home = cargo.join("home");
    let registry = cargo.join("registry-source");
    std::fs::create_dir_all(&home).expect("private test Cargo home");
    std::fs::create_dir_all(&registry).expect("private test Cargo registry");
    std::fs::write(home.join("config.toml"), "[net]\noffline = true\n")
        .expect("private test Cargo config");
    audited_cargo_command(
        &cargo_execution_root(root).expect("isolated Cargo execution root"),
        &home,
        &registry,
        compiler,
    )
    .expect("isolated Cargo command")
}

fn fixture(source: &str) -> PathBuf {
    let root = crate::test_support::reserve_temp_directory("wizard-api-fixture");
    std::fs::create_dir_all(root.join("libs/fixture/src")).expect("fixture directory");
    std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
    std::fs::write(root.join("libs/fixture/src/main.rs"), source).expect("fixture source");
    root
}

fn compile_fixture(root: &Path, extra: &[&str]) -> String {
    let mut command = Command::new("rustc");
    command.current_dir(root).args([
        "--edition=2024",
        "libs/fixture/src/main.rs",
        "--emit=metadata",
        "--error-format=short",
        "-o",
        "fixture.rmeta",
    ]);
    command.args(extra);
    let result = command.output().expect("compile fixture");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stderr).expect("rustc diagnostics are UTF-8")
}

fn compile_wost_fixture(root: &Path) -> String {
    let result = Command::new("rustc")
        .current_dir(root)
        .args([
            "--edition=2024",
            "--crate-name",
            "wost",
            "wost/src/lib.rs",
            "--crate-type",
            "lib",
            "--emit=metadata",
            "--error-format=short",
            "-o",
            "wost.rmeta",
        ])
        .output()
        .expect("compile wost fixture");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stderr).expect("rustc diagnostics are UTF-8")
}
