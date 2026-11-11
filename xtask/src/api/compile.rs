use super::*;

pub(super) fn cargo_encoded_rustflags(
    target: Option<&str>,
    source_root: &Path,
    toolchain_root: &Path,
    cargo_source_root: &Path,
) -> Result<std::ffi::OsString> {
    match target {
        None => Ok(std::ffi::OsString::new()),
        Some(crate::paths::KERNEL_TARGET) => {
            crate::cargo::kernel_encoded_rustflags(source_root, toolchain_root, cargo_source_root)
        }
        Some(target) => bail!("API audit has no exact rustflags contract for target `{target}`"),
    }
}

pub(super) fn audited_cargo_command(
    execution_root: &Path,
    cargo_home: &Path,
    registry_source: &Path,
    compiler: &CompilerIdentity,
) -> Result<Command> {
    let mut command = base_cargo_command(execution_root, cargo_home, compiler);
    let registry = format!("{:?}", registry_source.to_string_lossy());
    command
        .args([
            "--config",
            "source.crates-io.replace-with=\"api-audit-registry\"",
            "--config",
        ])
        .arg(format!("source.api-audit-registry.directory={registry}"));
    Ok(command)
}

// An inherited value could change the pinned driver's tools or flags.
const AMBIENT_COMPILER_VARIABLES: [&str; 18] = [
    "CPATH",
    "C_INCLUDE_PATH",
    "CPLUS_INCLUDE_PATH",
    "LD_RUN_PATH",
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "CCC_OVERRIDE_OPTIONS",
    "CC",
    "CFLAGS",
    "AR",
    "TARGET_CC",
    "HOST_CC",
    "CRATE_CC_NO_DEFAULTS",
    "CXX",
    "CXXFLAGS",
    "ARFLAGS",
    "RANLIB",
    "CC_ENABLE_DEBUG_OUTPUT",
];
pub(super) const AMBIENT_COMPILER_PREFIXES: [&str; 8] = [
    "CC_",
    "CFLAGS_",
    "CXX_",
    "CXXFLAGS_",
    "AR_",
    "ARFLAGS_",
    "TARGET_",
    "HOST_",
];

// gcc would take `ld` from GCC_EXEC_PREFIX before COMPILER_PATH, and libraries from LIBRARY_PATH.
pub(super) fn base_cargo_command(
    execution_root: &Path,
    cargo_home: &Path,
    compiler: &CompilerIdentity,
) -> Command {
    let mut command = Command::new(env!("CARGO"));
    remove_ambient_cargo_semantics(&mut command, std::env::vars_os().map(|(name, _)| name));
    for name in AMBIENT_COMPILER_VARIABLES {
        command.env_remove(name);
    }
    command
        .current_dir(execution_root)
        .env("CARGO_HOME", cargo_home)
        .env("PATH", &compiler.tool_path)
        .env_remove("GCC_EXEC_PREFIX")
        .env_remove("LIBRARY_PATH")
        .env("RUSTC", &compiler.path)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .args(["--config"])
        .arg(format!(
            "target.{}.linker={:?}",
            compiler.host_target,
            compiler.host_linker.to_string_lossy()
        ));
    if let Some(link_editor) = &compiler.link_editor {
        command.env("COMPILER_PATH", &link_editor.search_directory);
    }
    command
}

pub(super) fn compile_graphs(
    workspace: &AuditWorkspace,
    round: usize,
    expected: &BTreeMap<PathBuf, [u8; 32]>,
    plan: &AuditPlan,
    pristine_build_authority: Option<&[BTreeMap<PathBuf, [u8; 32]>]>,
) -> Result<Vec<Compilation>> {
    run_bounded_in_order(
        plan.compilations.len(),
        CONCURRENT_GRAPHS,
        |graph| compiles_alone(&plan.compilations[graph]),
        |graph| {
            let compilation = &plan.compilations[graph];
            let compiled = compile_graph(
                workspace,
                round,
                graph,
                &plan.compiler,
                compilation,
                expected,
            )?;
            if let Some(pristine) = pristine_build_authority {
                let expected = pristine
                    .get(graph)
                    .context("a compiler graph has no pristine build-script baseline")?;
                require_build_script_authority_unchanged(
                    &graph_build_directory(&workspace.build, graph)?,
                    expected,
                    &compilation_label(compilation),
                )?;
            }
            Ok(compiled)
        },
    )
}

const CONCURRENT_GRAPHS: usize = 4;

pub(super) fn compiles_alone(plan: &CompilationPlan) -> bool {
    matches!(&plan.mode, CompilationMode::TestHarness(target) if target.package == "xtask")
}

pub(super) fn run_bounded_in_order<T: Send>(
    count: usize,
    workers: usize,
    alone: impl Fn(usize) -> bool,
    job: impl Fn(usize) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let (solo, shared): (Vec<usize>, Vec<usize>) = (0..count).partition(|&index| alone(index));
    let mut slots = (0..count).map(|_| None).collect::<Vec<Option<T>>>();
    for batch in shared.chunks(workers.max(1)) {
        let completed = std::thread::scope(|scope| {
            let job = &job;
            let handles = batch
                .iter()
                .map(|&index| (index, scope.spawn(move || job(index))))
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|(index, handle)| {
                    let outcome = handle.join().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("API-audit graph {index} worker panicked"))
                    });
                    (index, outcome)
                })
                .collect::<Vec<_>>()
        });
        for (index, outcome) in completed {
            slots[index] = Some(outcome?);
        }
    }
    for index in solo {
        slots[index] = Some(job(index)?);
    }
    slots
        .into_iter()
        .map(|slot| slot.context("an API-audit graph was never compiled"))
        .collect()
}

fn compile_graph(
    workspace: &AuditWorkspace,
    round: usize,
    graph: usize,
    compiler: &CompilerIdentity,
    plan: &CompilationPlan,
    expected: &BTreeMap<PathBuf, [u8; 32]>,
) -> Result<Compilation> {
    let args = cargo_arguments(plan);
    let build = graph_build_directory(&workspace.build, graph)?;
    let mut command = audited_cargo_command(
        &workspace.execution_root,
        &workspace.cargo_home,
        &workspace.registry_source,
        compiler,
    )?;
    command
        .env("CARGO_TARGET_DIR", build)
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            cargo_encoded_rustflags(
                plan.pass.target,
                &workspace.root,
                &compiler.input_authority.sysroot,
                &workspace.registry_source,
            )?,
        )
        .env("CARGO_BUILD_WARNINGS", "warn")
        .args(&args)
        .arg("--manifest-path")
        .arg(workspace.root.join("Cargo.toml"))
        .arg("--color=never");
    let selected_test = match &plan.mode {
        CompilationMode::TestHarness(target) => Some(target.clone()),
        CompilationMode::RegisteredPass | CompilationMode::Example(_) => None,
    };
    let captured = crate::bounded_process::output_streamed(
        command,
        COMPILATION_TIMEOUT,
        "API audit compilation",
        MAX_AUDIT_OUTPUT_BYTES,
        move |stdout| {
            cargo_compilation_stream(
                BufReader::new(stdout),
                selected_test,
                MAX_AUDIT_JSON_LINE_BYTES,
            )
        },
    )
    .with_context(|| {
        format!(
            "spawning API audit round {round} for {}",
            compilation_label(plan)
        )
    })?;
    workspace.require_copy_unchanged(expected)?;
    workspace.require_source_unchanged(&workspace.source_root)?;
    compiler.require_unchanged()?;
    if !captured.status.success() {
        bail!(
            "API audit round {round} failed to compile {} ({})\n{}{}",
            compilation_label(plan),
            captured.status,
            captured.value.failure_report(),
            String::from_utf8_lossy(&captured.stderr)
        );
    }
    let diagnostics = captured.value.finish()?;
    Ok(Compilation {
        graph,
        kind: plan.kind,
        diagnostics,
    })
}

pub(super) fn cargo_arguments(plan: &CompilationPlan) -> Vec<String> {
    let (command, selected) = match &plan.mode {
        CompilationMode::RegisteredPass => ("check", None),
        CompilationMode::Example(target) => ("check", Some(target)),
        CompilationMode::TestHarness(target) => ("test", Some(target)),
    };
    let mut args = vec![command.to_string()];
    if matches!(plan.mode, CompilationMode::TestHarness(_)) {
        args.push("--no-run".to_string());
    }
    args.extend([
        "--jobs=1".to_string(),
        "--offline".to_string(),
        "--locked".to_string(),
        "--message-format=json-diagnostic-short".to_string(),
    ]);
    if let Some(target) = selected {
        args.extend(["--package".to_string(), target.package.clone()]);
        args.extend(target_selector(target));
    } else {
        args.extend(crate::cargo::package_args(plan.pass.packages));
    }
    args.extend(["--target".to_string(), plan.target.clone()]);
    if let Some(features) = plan.pass.features {
        args.extend(["--features".to_string(), features.to_string()]);
    }
    args
}

fn target_selector(target: &CargoTarget) -> Vec<String> {
    match target.kind {
        CargoTargetKind::Library => vec!["--lib".to_string()],
        CargoTargetKind::Bin => vec!["--bin".to_string(), target.name.clone()],
        CargoTargetKind::Example => vec!["--example".to_string(), target.name.clone()],
        CargoTargetKind::Test => vec!["--test".to_string(), target.name.clone()],
        CargoTargetKind::Bench => vec!["--bench".to_string(), target.name.clone()],
        CargoTargetKind::CustomBuild => unreachable!("build scripts have no Cargo selector"),
    }
}

fn compilation_label(plan: &CompilationPlan) -> String {
    match &plan.mode {
        CompilationMode::RegisteredPass => plan.pass.label(),
        CompilationMode::Example(target) => {
            format!("{} example {}", target.package, target.name)
        }
        CompilationMode::TestHarness(target) => format!(
            "{} {} {} test harness",
            target.package,
            target.kind.metadata_name(),
            target.name
        ),
    }
}

#[derive(Debug)]
pub(super) struct CargoCompilationEvidence {
    diagnostics: Vec<MarkerDiagnostic>,
    retained_bytes: usize,
    selected_test: Option<CargoTarget>,
    selected_artifact: bool,
    test_artifact: bool,
    failures: Vec<String>,
    failure_bytes: usize,
}

impl CargoCompilationEvidence {
    pub(super) fn new(selected_test: Option<CargoTarget>) -> Self {
        Self {
            diagnostics: Vec::new(),
            retained_bytes: 0,
            selected_test,
            selected_artifact: false,
            test_artifact: false,
            failures: Vec::new(),
            failure_bytes: 0,
        }
    }

    pub(super) fn observe(&mut self, line: &[u8]) -> Result<()> {
        if line.iter().all(u8::is_ascii_whitespace) {
            return Ok(());
        }
        let value = crate::json::parse_json(line, "Cargo compiler JSON message")?;
        let message = crate::json::json_object(&value, "Cargo compiler JSON message")?;
        let reason = crate::json::json_string(
            crate::json::json_field(message, "reason", "Cargo compiler JSON message")?,
            "Cargo compiler JSON `reason`",
        )?;
        match reason {
            "compiler-message" => self.observe_compiler_message(message),
            "compiler-artifact" => self.observe_compiler_artifact(message),
            _ => Ok(()),
        }
    }

    fn observe_compiler_message(
        &mut self,
        message: &BTreeMap<String, crate::json::JsonValue>,
    ) -> Result<()> {
        let package_id = crate::json::json_string(
            crate::json::json_field(message, "package_id", "Cargo compiler message")?,
            "Cargo compiler message `package_id`",
        )?;
        let target = crate::json::json_object(
            crate::json::json_field(message, "target", "Cargo compiler message")?,
            "Cargo compiler message target",
        )?;
        let compiler_message = crate::json::json_object(
            crate::json::json_field(message, "message", "Cargo compiler message")?,
            "Cargo compiler message",
        )?;
        let diagnostic_message = crate::json::json_string(
            crate::json::json_field(compiler_message, "message", "Cargo compiler message")?,
            "Cargo compiler diagnostic `message`",
        )?;
        let level = crate::json::json_string(
            crate::json::json_field(compiler_message, "level", "Cargo compiler message")?,
            "Cargo compiler diagnostic `level`",
        )?;
        let Some(id) = diagnostic_marker_id(diagnostic_message)? else {
            if level == "error" {
                self.retain_failure(diagnostic_message)?;
            }
            return Ok(());
        };
        ensure!(
            level == "warning",
            "API marker diagnostic has compiler level `{level}` instead of `warning`"
        );
        let code = crate::json::json_object(
            crate::json::json_field(compiler_message, "code", "Cargo compiler message")?,
            "API marker diagnostic code",
        )?;
        let code = crate::json::json_string(
            crate::json::json_field(code, "code", "API marker diagnostic code")?,
            "API marker diagnostic code name",
        )?;
        ensure!(
            code == "deprecated",
            "API marker diagnostic has compiler code `{code}` instead of `deprecated`"
        );
        let primary = primary_diagnostic_span(compiler_message)?;
        let primary_location = cargo_diagnostic_location(primary, "primary API diagnostic span")?;
        let immediate_expansion = immediate_diagnostic_expansion(primary)?;
        let target_name = crate::json::json_string(
            crate::json::json_field(target, "name", "Cargo compiler message target")?,
            "Cargo compiler message target `name`",
        )?;
        let kinds = crate::json::json_string_array(
            crate::json::json_field(target, "kind", "Cargo compiler message target")?,
            "Cargo compiler message target `kind`",
        )?;
        let target_source = PathBuf::from(crate::json::json_string(
            crate::json::json_field(target, "src_path", "Cargo compiler message target")?,
            "Cargo compiler message target `src_path`",
        )?);
        ensure!(
            target_source.is_absolute(),
            "Cargo compiler message target source is not absolute"
        );
        let diagnostic = MarkerDiagnostic {
            id,
            origin: CargoDiagnosticOrigin {
                package_id: package_id.to_string(),
                target_name: target_name.to_string(),
                target_kind: CargoTargetKind::parse(&kinds)?,
                target_source,
            },
            reported: primary_location.reported,
            line: primary_location.line,
            column: primary_location.column,
            immediate_expansion,
        };
        self.retain_marker_with_limits(
            diagnostic,
            MAX_AUDIT_RETAINED_MARKERS,
            MAX_AUDIT_OUTPUT_BYTES,
        )
    }

    fn observe_compiler_artifact(
        &mut self,
        message: &BTreeMap<String, crate::json::JsonValue>,
    ) -> Result<()> {
        let Some(selected) = self.selected_test.as_ref() else {
            return Ok(());
        };
        let package_id = crate::json::json_string(
            crate::json::json_field(message, "package_id", "Cargo compiler artifact")?,
            "Cargo compiler artifact `package_id`",
        )?;
        let target = crate::json::json_object(
            crate::json::json_field(message, "target", "Cargo compiler artifact")?,
            "Cargo compiler artifact target",
        )?;
        let target_name = crate::json::json_string(
            crate::json::json_field(target, "name", "Cargo compiler artifact target")?,
            "Cargo compiler artifact target `name`",
        )?;
        let kinds = crate::json::json_string_array(
            crate::json::json_field(target, "kind", "Cargo compiler artifact target")?,
            "Cargo compiler artifact target `kind`",
        )?;
        if package_id != selected.package_id
            || target_name != selected.name
            || CargoTargetKind::parse(&kinds)? != selected.kind
        {
            return Ok(());
        }
        self.selected_artifact = true;
        let profile = crate::json::json_object(
            crate::json::json_field(message, "profile", "Cargo compiler artifact")?,
            "Cargo compiler artifact profile",
        )?;
        let test =
            match crate::json::json_field(profile, "test", "Cargo compiler artifact profile")? {
                crate::json::JsonValue::Bool(test) => *test,
                _ => bail!("Cargo compiler artifact profile `test` is not a Boolean"),
            };
        self.test_artifact |= test;
        Ok(())
    }

    pub(super) fn retain_marker_with_limits(
        &mut self,
        diagnostic: MarkerDiagnostic,
        maximum_markers: usize,
        maximum_bytes: usize,
    ) -> Result<()> {
        let count = self
            .diagnostics
            .len()
            .checked_add(1)
            .context("retained API marker count overflowed")?;
        ensure!(
            count <= maximum_markers,
            "retained API marker count exceeded {maximum_markers} records"
        );
        let retained = self
            .retained_bytes
            .checked_add(marker_retained_bytes(&diagnostic)?)
            .context("retained API marker evidence length overflowed")?;
        ensure!(
            retained <= maximum_bytes,
            "retained API marker evidence exceeded {maximum_bytes} bytes"
        );
        self.retained_bytes = retained;
        self.diagnostics.push(diagnostic);
        Ok(())
    }

    fn retain_failure(&mut self, message: &str) -> Result<()> {
        let bytes = message
            .len()
            .checked_add(1)
            .and_then(|bytes| self.failure_bytes.checked_add(bytes))
            .context("API compiler failure diagnostics overflowed")?;
        ensure!(
            bytes <= MAX_AUDIT_FAILURE_BYTES,
            "API compiler failure diagnostics exceeded {MAX_AUDIT_FAILURE_BYTES} bytes"
        );
        self.failure_bytes = bytes;
        self.failures.push(message.to_string());
        Ok(())
    }

    fn failure_report(&self) -> String {
        if self.failures.is_empty() {
            String::new()
        } else {
            format!("{}\n", self.failures.join("\n"))
        }
    }

    pub(super) fn finish(self) -> Result<Vec<MarkerDiagnostic>> {
        if let Some(selected) = &self.selected_test {
            ensure!(
                self.selected_artifact,
                "Cargo emitted no artifact for selected target `{}:{}`",
                selected.package,
                selected.name
            );
            ensure!(
                self.test_artifact,
                "Cargo target `{}:{}` uses a custom test harness, which the API audit cannot classify",
                selected.package,
                selected.name
            );
        }
        Ok(self.diagnostics)
    }
}

fn primary_diagnostic_span(
    compiler_message: &BTreeMap<String, crate::json::JsonValue>,
) -> Result<&BTreeMap<String, crate::json::JsonValue>> {
    let spans = crate::json::json_array(
        crate::json::json_field(compiler_message, "spans", "Cargo compiler message")?,
        "Cargo compiler diagnostic `spans`",
    )?;
    let primary = spans
        .iter()
        .map(|span| {
            let span = crate::json::json_object(span, "Cargo compiler diagnostic span")?;
            let primary = match crate::json::json_field(
                span,
                "is_primary",
                "Cargo compiler diagnostic span",
            )? {
                crate::json::JsonValue::Bool(primary) => *primary,
                _ => bail!("Cargo compiler diagnostic `is_primary` is not a Boolean"),
            };
            Ok(primary.then_some(span))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let [primary] = primary.as_slice() else {
        bail!(
            "API marker diagnostic has {} primary spans instead of exactly one",
            primary.len()
        );
    };
    Ok(*primary)
}

fn immediate_diagnostic_expansion(
    primary: &BTreeMap<String, crate::json::JsonValue>,
) -> Result<Option<DiagnosticExpansion>> {
    match crate::json::json_field(primary, "expansion", "primary API diagnostic span")? {
        crate::json::JsonValue::Null => Ok(None),
        expansion @ crate::json::JsonValue::Object(_) => {
            let expansion =
                crate::json::json_object(expansion, "primary API diagnostic expansion")?;
            let macro_decl_name = crate::json::json_string(
                crate::json::json_field(
                    expansion,
                    "macro_decl_name",
                    "primary API diagnostic expansion",
                )?,
                "primary API diagnostic expansion `macro_decl_name`",
            )?;
            let span = crate::json::json_object(
                crate::json::json_field(expansion, "span", "primary API diagnostic expansion")?,
                "primary API diagnostic expansion span",
            )?;
            let terminal = match crate::json::json_field(
                span,
                "expansion",
                "primary API diagnostic expansion span",
            )? {
                crate::json::JsonValue::Null => true,
                crate::json::JsonValue::Object(_) => false,
                _ => bail!(
                    "primary API diagnostic expansion span `expansion` is neither null nor an object"
                ),
            };
            Ok(Some(DiagnosticExpansion {
                location: cargo_diagnostic_location(span, "primary API diagnostic expansion span")?,
                terminal,
                macro_decl_name: macro_decl_name.to_string(),
            }))
        }
        _ => bail!("primary API diagnostic `expansion` is neither null nor an object"),
    }
}

pub(super) fn marker_retained_bytes(diagnostic: &MarkerDiagnostic) -> Result<usize> {
    let mut bytes = std::mem::size_of::<MarkerDiagnostic>();
    for field in [
        diagnostic.origin.package_id.len(),
        diagnostic.origin.target_name.len(),
        diagnostic
            .origin
            .target_source
            .as_os_str()
            .as_encoded_bytes()
            .len(),
        diagnostic.reported.len(),
    ] {
        bytes = bytes
            .checked_add(field)
            .context("retained API marker evidence length overflowed")?;
    }
    if let Some(expansion) = &diagnostic.immediate_expansion {
        for field in [
            expansion.location.reported.len(),
            expansion.macro_decl_name.len(),
        ] {
            bytes = bytes
                .checked_add(field)
                .context("retained API marker evidence length overflowed")?;
        }
    }
    Ok(bytes)
}

pub(super) fn cargo_compilation_stream<R: BufRead>(
    mut reader: R,
    selected_test: Option<CargoTarget>,
    maximum_line: usize,
) -> Result<CargoCompilationEvidence> {
    let mut evidence = CargoCompilationEvidence::new(selected_test);
    let mut line = Vec::new();
    loop {
        let read = read_bounded_json_line(&mut reader, &mut line, maximum_line)?;
        if read == 0 {
            return Ok(evidence);
        }
        evidence.observe(&line)?;
    }
}

fn read_bounded_json_line<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    maximum: usize,
) -> Result<usize> {
    line.clear();
    loop {
        let available = reader
            .fill_buf()
            .context("reading streamed Cargo compiler JSON")?;
        if available.is_empty() {
            ensure!(
                line.is_empty(),
                "Cargo compiler JSON ended without a record terminator"
            );
            return Ok(0);
        }
        let end = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        let length = line
            .len()
            .checked_add(end)
            .context("Cargo compiler JSON record length overflowed")?;
        ensure!(
            length <= maximum,
            "Cargo compiler JSON record exceeded {maximum} bytes"
        );
        let terminated = available[end - 1] == b'\n';
        line.extend_from_slice(&available[..end]);
        reader.consume(end);
        if terminated {
            return Ok(line.len());
        }
    }
}

pub(super) fn graph_build_directory(build: &Path, graph: usize) -> Result<PathBuf> {
    let directory = build.join(format!("graph-{graph}"));
    let directory = crate::mutate::workspace::prepare_real_child(
        build,
        &directory,
        "API audit graph build directory",
    )?;
    Ok(crate::cargo::child_process_path(&directory))
}

pub(super) fn build_script_authority_snapshot(root: &Path) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutate::workspace::canonical_existing_real_directory(
        root,
        "API audit compiler-graph build directory",
    )?;
    let mut snapshot = BTreeMap::new();
    collect_build_script_authority(root, root, &mut snapshot)?;
    Ok(snapshot)
}

pub(super) fn require_build_script_authority_unchanged(
    root: &Path,
    expected: &BTreeMap<PathBuf, [u8; 32]>,
    label: &str,
) -> Result<()> {
    ensure!(
        build_script_authority_snapshot(root)? == *expected,
        "{label} changed build-script output or OUT_DIR authority after source instrumentation"
    );
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BuildScriptAuthorityPath {
    Output,
    OutRoot,
    OutEntry,
}

fn collect_build_script_authority(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    crate::mutate::workspace::canonical_existing_real_directory(
        directory,
        "API audit build-script authority directory",
    )?;
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        let relative = entry
            .path()
            .strip_prefix(root)
            .context("build-script authority escaped its compiler graph")?
            .to_path_buf();
        let authority = build_script_authority_path(&relative);
        if kind.is_dir() {
            ensure!(
                authority != Some(BuildScriptAuthorityPath::Output),
                "build-script output {} is not a regular file",
                entry.path().display()
            );
            collect_build_script_authority(root, &entry.path(), snapshot)?;
        } else if kind.is_file() {
            ensure!(
                authority != Some(BuildScriptAuthorityPath::OutRoot),
                "build-script OUT_DIR {} is not a directory",
                entry.path().display()
            );
            if authority.is_some() {
                snapshot_marker_free_file(
                    root,
                    &entry.path(),
                    snapshot,
                    "build-script compiler input",
                )?;
            }
        } else if authority.is_some() {
            bail!(
                "build-script authority {} is not a regular file or directory",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn build_script_authority_path(relative: &Path) -> Option<BuildScriptAuthorityPath> {
    let components = relative
        .components()
        .map(|component| component.as_os_str())
        .collect::<Vec<_>>();
    components
        .iter()
        .enumerate()
        .find_map(|(index, component)| {
            if component != &"build" || index + 2 >= components.len() {
                return None;
            }
            let authority = &components[index + 2];
            if authority == &"output" && index + 3 == components.len() {
                Some(BuildScriptAuthorityPath::Output)
            } else if authority == &"out" && index + 3 == components.len() {
                Some(BuildScriptAuthorityPath::OutRoot)
            } else if authority == &"out" {
                Some(BuildScriptAuthorityPath::OutEntry)
            } else {
                None
            }
        })
}
