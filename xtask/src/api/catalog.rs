use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CargoTargetKind {
    Library,
    Bin,
    Example,
    Test,
    Bench,
    CustomBuild,
}

impl CargoTargetKind {
    pub(super) fn parse(kinds: &[&str]) -> Result<Self> {
        let [kind] = kinds else {
            bail!(
                "Cargo metadata target has ambiguous kinds: {}",
                kinds.join(", ")
            );
        };
        match *kind {
            "lib" => Ok(Self::Library),
            "bin" => Ok(Self::Bin),
            "example" => Ok(Self::Example),
            "test" => Ok(Self::Test),
            "bench" => Ok(Self::Bench),
            "custom-build" => Ok(Self::CustomBuild),
            _ => bail!("Cargo metadata target has unsupported kind `{kind}`"),
        }
    }

    pub(super) fn metadata_name(self) -> &'static str {
        match self {
            Self::Library => "lib",
            Self::Bin => "bin",
            Self::Example => "example",
            Self::Test => "test",
            Self::Bench => "bench",
            Self::CustomBuild => "custom-build",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct CargoTarget {
    pub(super) package: String,
    pub(super) package_id: String,
    pub(super) name: String,
    pub(super) relative: PathBuf,
    pub(super) kind: CargoTargetKind,
    pub(super) test: bool,
    pub(super) executable: bool,
}

impl CargoTarget {
    pub(super) fn context(&self) -> ModuleContext {
        let availability = match self.kind {
            CargoTargetKind::Test | CargoTargetKind::Bench => Availability::Verification,
            _ => Availability::Production,
        };
        let language_main = match self.kind {
            CargoTargetKind::Bin | CargoTargetKind::CustomBuild => true,
            CargoTargetKind::Example => self.executable,
            CargoTargetKind::Library | CargoTargetKind::Test | CargoTargetKind::Bench => false,
        };
        ModuleContext {
            scope: Vec::new(),
            availability,
            package: self.package.clone(),
            crate_name: self.name.replace('-', "_"),
            target_root: self.relative.clone(),
            target_kind: self.kind,
            language_main,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct ResolvedDependency {
    pub(super) name: String,
    pub(super) package_id: String,
    pub(super) normal: bool,
    pub(super) build: bool,
    pub(super) dev: bool,
}

#[derive(Debug)]
pub(super) struct DependencyBuildScript {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) registry: bool,
}

#[derive(Debug)]
pub(super) struct CargoCatalog {
    pub(super) packages: BTreeMap<String, Vec<CargoTarget>>,
    pub(super) resolved: BTreeMap<String, Vec<ResolvedDependency>>,
}

impl CargoCatalog {
    pub(super) fn discover(
        workspace: &AuditWorkspace,
        expected: &BTreeMap<PathBuf, [u8; 32]>,
        pass: gate::CargoPass,
        compiler: &CompilerIdentity,
    ) -> Result<Self> {
        let manifest = workspace.root.join("Cargo.toml");
        let mut command = audited_cargo_command(
            &workspace.execution_root,
            &workspace.cargo_home,
            &workspace.registry_source,
            compiler,
        )?;
        command
            .env("CARGO_BUILD_WARNINGS", "warn")
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                cargo_encoded_rustflags(
                    pass.target,
                    &workspace.root,
                    &compiler.input_authority.sysroot,
                    &workspace.registry_source,
                )?,
            )
            .args([
                "metadata",
                "--format-version=1",
                "--offline",
                "--locked",
                "--color=never",
                "--manifest-path",
            ])
            .arg(&manifest)
            .args([
                "--filter-platform",
                pass.target.unwrap_or(&compiler.host_target),
            ]);
        if let Some(features) = pass.features {
            command.args(["--features", features]);
        }
        let result = crate::bounded_process::output_with_limit(
            command,
            METADATA_TIMEOUT,
            "Cargo metadata for the API audit",
            MAX_AUDIT_OUTPUT_BYTES,
        )
        .context("spawning Cargo metadata for the API audit")?;
        workspace.require_copy_unchanged(expected)?;
        workspace.require_source_unchanged(&workspace.source_root)?;
        workspace.require_cargo_inputs_unchanged()?;
        compiler.require_unchanged()?;
        ensure!(
            result.status.success(),
            "Cargo metadata for the API audit failed ({}):\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        );
        Self::parse(
            &workspace.root,
            &workspace.registry_source,
            &result.stdout,
            pass.packages,
            pass.target.is_none(),
        )
    }

    pub(super) fn parse(
        root: &Path,
        registry_source: &Path,
        bytes: &[u8],
        selected_packages: &[&str],
        include_root_dev_dependencies: bool,
    ) -> Result<Self> {
        reject_custom_harness_manifests(root)?;
        let value = crate::json::parse_json(bytes, "Cargo metadata JSON")?;
        let document = crate::json::json_object(&value, "Cargo metadata JSON")?;
        ensure!(
            crate::json::json_u64(
                crate::json::json_field(document, "version", "Cargo metadata JSON")?,
                "Cargo metadata `version`"
            )? == 1,
            "Cargo metadata has an unsupported format version"
        );

        let declared_root = PathBuf::from(crate::json::json_string(
            crate::json::json_field(document, "workspace_root", "Cargo metadata JSON")?,
            "Cargo metadata `workspace_root`",
        )?);
        let canonical_root = std::fs::canonicalize(root)
            .with_context(|| format!("canonicalizing API audit root {}", root.display()))?;
        ensure!(
            std::fs::canonicalize(&declared_root).with_context(|| format!(
                "canonicalizing Cargo metadata workspace root {}",
                declared_root.display()
            ))? == canonical_root,
            "Cargo metadata describes a different workspace root"
        );

        let members = crate::json::json_string_array(
            crate::json::json_field(document, "workspace_members", "Cargo metadata JSON")?,
            "Cargo metadata `workspace_members`",
        )?
        .into_iter()
        .collect::<BTreeSet<_>>();
        let package_values = crate::json::json_array(
            crate::json::json_field(document, "packages", "Cargo metadata JSON")?,
            "Cargo metadata `packages`",
        )?;
        let mut package_names = BTreeMap::new();
        let mut procedural_macros = BTreeMap::new();
        let mut dependency_build_scripts = BTreeMap::new();
        for package_value in package_values {
            let package = crate::json::json_object(package_value, "Cargo metadata package")?;
            let id = crate::json::json_string(
                crate::json::json_field(package, "id", "Cargo metadata package")?,
                "Cargo metadata package `id`",
            )?;
            let name = crate::json::json_string(
                crate::json::json_field(package, "name", "Cargo metadata package")?,
                "Cargo metadata package `name`",
            )?;
            ensure!(
                package_names
                    .insert(id.to_string(), name.to_string())
                    .is_none(),
                "Cargo metadata contains duplicate package ID `{id}`"
            );
            let targets = crate::json::json_array(
                crate::json::json_field(package, "targets", "Cargo metadata package")?,
                "Cargo metadata package `targets`",
            )?;
            let mut custom_build = false;
            for target in targets {
                let target = crate::json::json_object(target, "Cargo metadata target")?;
                let kinds = crate::json::json_string_array(
                    crate::json::json_field(target, "kind", "Cargo metadata target")?,
                    "Cargo metadata target `kind`",
                )?;
                if kinds.contains(&"proc-macro") {
                    procedural_macros.insert(id.to_string(), name.to_string());
                }
                custom_build |= kinds.contains(&"custom-build");
            }
            if custom_build && !members.contains(id) {
                let version = crate::json::json_string(
                    crate::json::json_field(package, "version", "Cargo metadata package")?,
                    "Cargo metadata package `version`",
                )?;
                let registry = matches!(
                    crate::json::json_field(package, "source", "Cargo metadata package")?,
                    crate::json::JsonValue::String(source)
                        if source == "registry+https://github.com/rust-lang/crates.io-index"
                );
                dependency_build_scripts.insert(
                    id.to_string(),
                    DependencyBuildScript {
                        name: name.to_string(),
                        version: version.to_string(),
                        registry,
                    },
                );
            }
        }

        let resolved = parse_resolved_dependencies(document, &package_names)?;
        let workspace_ids = members
            .iter()
            .map(|id| {
                package_names
                    .get(*id)
                    .map(|name| (name.clone(), (*id).to_string()))
                    .with_context(|| format!("workspace member `{id}` has no package definition"))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let root_ids = selected_packages
            .iter()
            .map(|name| {
                workspace_ids.get(*name).cloned().with_context(|| {
                    format!("registered Cargo package `{name}` is absent from metadata")
                })
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let active = active_resolved_packages(&resolved, &root_ids, include_root_dev_dependencies)?;
        reject_reserved_dependency_roots(
            &resolved,
            &active,
            &root_ids,
            include_root_dev_dependencies,
            &package_names,
        )?;
        if let Some((_, name)) = procedural_macros
            .iter()
            .find(|(id, _)| active.contains(*id))
        {
            bail!(
                "active procedural-macro package `{name}` is outside the API audit's syntactic trust boundary"
            );
        }
        validate_dependency_build_scripts(root, &active, &dependency_build_scripts)?;
        validate_active_external_package_roots(registry_source, package_values, &members, &active)?;

        let mut packages = BTreeMap::new();
        for package_value in package_values {
            let package = crate::json::json_object(package_value, "Cargo metadata package")?;
            let id = crate::json::json_string(
                crate::json::json_field(package, "id", "Cargo metadata package")?,
                "Cargo metadata package `id`",
            )?;
            let name = crate::json::json_string(
                crate::json::json_field(package, "name", "Cargo metadata package")?,
                "Cargo metadata package `name`",
            )?;
            let target_values = crate::json::json_array(
                crate::json::json_field(package, "targets", "Cargo metadata package")?,
                "Cargo metadata package `targets`",
            )?;
            if !members.contains(id) {
                continue;
            }
            let mut targets = Vec::new();
            for target_value in target_values {
                let target = parse_cargo_target(&canonical_root, name, id, target_value)?;
                if active.contains(id) && target.kind == CargoTargetKind::CustomBuild {
                    validate_workspace_build_script(&canonical_root, name, &target)?;
                }
                targets.push(target);
            }
            ensure!(
                !targets.is_empty(),
                "Cargo workspace package `{name}` has no target"
            );
            ensure!(
                packages.insert(name.to_string(), targets).is_none(),
                "Cargo workspace contains duplicate package name `{name}`"
            );
        }
        ensure!(
            packages.len() == members.len(),
            "Cargo metadata omitted one or more workspace members"
        );
        Ok(Self { packages, resolved })
    }

    pub(super) fn targets(&self) -> impl Iterator<Item = &CargoTarget> {
        self.packages.values().flatten()
    }

    pub(super) fn package(&self, name: &str) -> Result<&[CargoTarget]> {
        self.packages
            .get(name)
            .map(Vec::as_slice)
            .with_context(|| format!("registered Cargo package `{name}` is absent from metadata"))
    }

    pub(super) fn has_direct_normal_dependency(
        &self,
        package: &str,
        dependency_name: &str,
        dependency_package: &str,
    ) -> bool {
        let Some(package_id) = self
            .packages
            .get(package)
            .and_then(|targets| targets.first())
            .map(|target| target.package_id.as_str())
        else {
            return false;
        };
        let Some(dependency_id) = self
            .packages
            .get(dependency_package)
            .and_then(|targets| targets.first())
            .map(|target| target.package_id.as_str())
        else {
            return false;
        };
        self.resolved
            .get(package_id)
            .into_iter()
            .flatten()
            .any(|dependency| {
                dependency.normal
                    && dependency.package_id == dependency_id
                    && dependency.name == dependency_name
            })
    }
}

fn validate_active_external_package_roots(
    registry_source: &Path,
    packages: &[crate::json::JsonValue],
    members: &BTreeSet<&str>,
    active: &BTreeSet<String>,
) -> Result<()> {
    for package in packages {
        let package = crate::json::json_object(package, "Cargo metadata package")?;
        let id = crate::json::json_string(
            crate::json::json_field(package, "id", "Cargo metadata package")?,
            "Cargo metadata package `id`",
        )?;
        if !active.contains(id) || members.contains(id) {
            continue;
        }
        let name = crate::json::json_string(
            crate::json::json_field(package, "name", "Cargo metadata package")?,
            "Cargo metadata package `name`",
        )?;
        let version = crate::json::json_string(
            crate::json::json_field(package, "version", "Cargo metadata package")?,
            "Cargo metadata package `version`",
        )?;
        for (label, value) in [("name", name), ("version", version)] {
            ensure!(
                !value.is_empty()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+')
                    }),
                "active registry package has unsafe {label} `{value}`"
            );
        }
        ensure!(
            matches!(
                crate::json::json_field(package, "source", "Cargo metadata package")?,
                crate::json::JsonValue::String(source)
                    if source == "registry+https://github.com/rust-lang/crates.io-index"
            ),
            "active non-workspace package {name} {version} is not a checksum-bound crates.io package"
        );

        let registry_source = crate::mutate::workspace::canonical_existing_real_directory(
            registry_source,
            "API audit materialized registry source",
        )?;
        let package_root = registry_source.join(format!("{name}-{version}"));
        let package_root = crate::mutate::workspace::canonical_existing_real_directory(
            &package_root,
            "API audit materialized registry package",
        )?;
        ensure!(
            package_root.starts_with(&registry_source),
            "active registry package {name} {version} escaped the materialized registry source"
        );
        let manifest = PathBuf::from(crate::json::json_string(
            crate::json::json_field(package, "manifest_path", "Cargo metadata package")?,
            "Cargo metadata package `manifest_path`",
        )?);
        let manifest = std::fs::canonicalize(&manifest).with_context(|| {
            format!(
                "canonicalizing Cargo package manifest {}",
                manifest.display()
            )
        })?;
        ensure!(
            manifest == package_root.join("Cargo.toml"),
            "active package {name} {version} is outside the materialized registry source"
        );
        crate::mutate::workspace::require_direct_file(
            &package_root,
            &manifest,
            "API audit materialized registry manifest",
        )?;

        let targets = crate::json::json_array(
            crate::json::json_field(package, "targets", "Cargo metadata package")?,
            "Cargo metadata package `targets`",
        )?;
        ensure!(
            !targets.is_empty(),
            "active registry package {name} {version} has no compiler target"
        );
        for target in targets {
            let target = crate::json::json_object(target, "Cargo metadata target")?;
            let source = PathBuf::from(crate::json::json_string(
                crate::json::json_field(target, "src_path", "Cargo metadata target")?,
                "Cargo metadata target `src_path`",
            )?);
            let source = std::fs::canonicalize(&source).with_context(|| {
                format!(
                    "canonicalizing Cargo dependency target {}",
                    source.display()
                )
            })?;
            ensure!(
                source.starts_with(&package_root),
                "active package {name} {version} has a compiler target outside the materialized registry source"
            );
            let parent = source
                .parent()
                .context("Cargo dependency target has no parent directory")?;
            crate::mutate::workspace::require_direct_file(
                parent,
                &source,
                "API audit registry target source",
            )?;
            ensure!(
                source
                    .extension()
                    .is_some_and(|extension| extension == "rs"),
                "active registry package {name} {version} has a non-Rust compiler target"
            );
        }
    }

    Ok(())
}

fn reject_custom_harness_manifests(root: &Path) -> Result<()> {
    let mut manifests = Vec::new();
    let workspace = root.join("Cargo.toml");
    if workspace.is_file() {
        manifests.push(workspace);
    }
    for directory in COPY_DIRECTORIES {
        let directory = root.join(directory);
        if directory.is_dir() {
            collect_named_files(&directory, "Cargo.toml", &mut manifests)?;
        }
    }
    for manifest in manifests {
        let text = read_bounded_authority_text(&manifest, "API audit manifest")?;
        ensure!(
            !text.contains("harness") && !text.contains("\\u") && !text.contains("\\U"),
            "{} contains a custom-harness or encoded TOML key outside the API audit's target model",
            manifest.display()
        );
    }
    Ok(())
}

fn collect_named_files(directory: &Path, name: &str, files: &mut Vec<PathBuf>) -> Result<()> {
    crate::mutate::workspace::canonical_existing_real_directory(
        directory,
        "API audit manifest directory",
    )?;
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            collect_named_files(&entry.path(), name, files)?;
        } else if kind.is_file() && entry.file_name() == name {
            files.push(entry.path());
        } else if !kind.is_file() {
            bail!(
                "API audit manifest tree {} is not a regular file",
                entry.path().display()
            );
        }
    }
    Ok(())
}

pub(super) fn parse_resolved_dependencies(
    document: &BTreeMap<String, crate::json::JsonValue>,
    package_names: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Vec<ResolvedDependency>>> {
    let resolution = crate::json::json_object(
        crate::json::json_field(document, "resolve", "Cargo metadata JSON")?,
        "Cargo metadata `resolve`",
    )?;
    let nodes = crate::json::json_array(
        crate::json::json_field(resolution, "nodes", "Cargo metadata resolution")?,
        "Cargo metadata resolution `nodes`",
    )?;
    let mut resolved = BTreeMap::new();
    for node in nodes {
        let node = crate::json::json_object(node, "Cargo metadata resolved node")?;
        let id = crate::json::json_string(
            crate::json::json_field(node, "id", "Cargo metadata resolved node")?,
            "Cargo metadata resolved node `id`",
        )?;
        ensure!(
            package_names.contains_key(id),
            "Cargo resolution names unknown package ID `{id}`"
        );
        let declared = crate::json::json_string_array(
            crate::json::json_field(node, "dependencies", "Cargo metadata resolved node")?,
            "Cargo metadata resolved node `dependencies`",
        )?
        .into_iter()
        .collect::<BTreeSet<_>>();
        let dependencies = crate::json::json_array(
            crate::json::json_field(node, "deps", "Cargo metadata resolved node")?,
            "Cargo metadata resolved node `deps`",
        )?;
        let mut parsed = Vec::new();
        let mut parsed_ids = BTreeSet::new();
        for dependency in dependencies {
            let dependency =
                crate::json::json_object(dependency, "Cargo metadata resolved dependency")?;
            let name = crate::json::json_string(
                crate::json::json_field(dependency, "name", "Cargo metadata resolved dependency")?,
                "Cargo metadata resolved dependency `name`",
            )?;
            let package_id = crate::json::json_string(
                crate::json::json_field(dependency, "pkg", "Cargo metadata resolved dependency")?,
                "Cargo metadata resolved dependency `pkg`",
            )?;
            ensure!(
                package_names.contains_key(package_id),
                "Cargo resolution dependency names unknown package ID `{package_id}`"
            );
            parsed_ids.insert(package_id);
            let kinds = crate::json::json_array(
                crate::json::json_field(
                    dependency,
                    "dep_kinds",
                    "Cargo metadata resolved dependency",
                )?,
                "Cargo metadata resolved dependency `dep_kinds`",
            )?;
            let mut normal = false;
            let mut build = false;
            let mut dev = false;
            for kind in kinds {
                let kind =
                    crate::json::json_object(kind, "Cargo metadata resolved dependency kind")?;
                match crate::json::json_field(
                    kind,
                    "kind",
                    "Cargo metadata resolved dependency kind",
                )? {
                    crate::json::JsonValue::Null => normal = true,
                    crate::json::JsonValue::String(kind) if kind == "build" => build = true,
                    crate::json::JsonValue::String(kind) if kind == "dev" => dev = true,
                    crate::json::JsonValue::String(kind) => {
                        bail!("Cargo metadata has unsupported dependency kind `{kind}`")
                    }
                    _ => bail!("Cargo metadata dependency kind is not null or a string"),
                }
            }
            ensure!(
                normal || build || dev,
                "Cargo metadata dependency `{name}` has no dependency kind"
            );
            parsed.push(ResolvedDependency {
                name: name.to_string(),
                package_id: package_id.to_string(),
                normal,
                build,
                dev,
            });
        }
        ensure!(
            declared == parsed_ids,
            "Cargo metadata dependency identities disagree for package `{id}`"
        );
        ensure!(
            resolved.insert(id.to_string(), parsed).is_none(),
            "Cargo metadata contains duplicate resolved node `{id}`"
        );
    }
    Ok(resolved)
}

pub(super) fn active_resolved_packages(
    resolved: &BTreeMap<String, Vec<ResolvedDependency>>,
    root_ids: &BTreeSet<String>,
    include_root_dev_dependencies: bool,
) -> Result<BTreeSet<String>> {
    let mut active = BTreeSet::new();
    let mut pending = VecDeque::new();
    for root in root_ids {
        ensure!(
            resolved.contains_key(root),
            "registered Cargo root `{root}` has no resolved dependency node"
        );
        active.insert(root.clone());
        pending.push_back(root.clone());
    }
    while let Some(package) = pending.pop_front() {
        for dependency in resolved
            .get(&package)
            .context("an active Cargo package lost its dependency node")?
        {
            let enabled = dependency.normal
                || dependency.build
                || include_root_dev_dependencies && root_ids.contains(&package) && dependency.dev;
            if enabled && active.insert(dependency.package_id.clone()) {
                pending.push_back(dependency.package_id.clone());
            }
        }
    }
    Ok(active)
}

pub(super) fn reject_reserved_dependency_roots(
    resolved: &BTreeMap<String, Vec<ResolvedDependency>>,
    active: &BTreeSet<String>,
    root_ids: &BTreeSet<String>,
    include_root_dev_dependencies: bool,
    package_names: &BTreeMap<String, String>,
) -> Result<()> {
    for package in active {
        for dependency in resolved
            .get(package)
            .context("an active Cargo package lost its dependency node")?
        {
            let enabled = dependency.normal
                || dependency.build
                || include_root_dev_dependencies && root_ids.contains(package) && dependency.dev;
            if enabled && RESERVED_LANGUAGE_ROOTS.contains(&dependency.name.as_str()) {
                let owner = package_names
                    .get(package)
                    .context("an active Cargo package has no package name")?;
                bail!(
                    "active Cargo package `{owner}` exposes a dependency under reserved language root `{}`",
                    dependency.name
                );
            }
        }
    }
    Ok(())
}

pub(super) fn validate_dependency_build_scripts(
    root: &Path,
    active: &BTreeSet<String>,
    scripts: &BTreeMap<String, DependencyBuildScript>,
) -> Result<()> {
    let active_scripts = scripts
        .iter()
        .filter(|(id, _)| active.contains(*id))
        .map(|(_, script)| script)
        .collect::<Vec<_>>();
    if active_scripts.is_empty() {
        return Ok(());
    }
    let locked = locked_registry_packages(&root.join("Cargo.lock"))?;
    for script in active_scripts {
        ensure!(
            script.registry,
            "active dependency build script {} {} is not a checksum-bound crates.io package",
            script.name,
            script.version
        );
        let checksums = locked
            .iter()
            .filter(|package| package.name == script.name && package.version == script.version)
            .map(|package| package.checksum.as_str())
            .collect::<Vec<_>>();
        let [checksum] = checksums.as_slice() else {
            bail!(
                "active dependency build script {} {} has {} Cargo.lock identities",
                script.name,
                script.version,
                checksums.len()
            );
        };
        ensure!(
            REVIEWED_DEPENDENCY_BUILD_SCRIPTS
                .iter()
                .any(|(name, version, reviewed_checksum)| {
                    script.name == *name
                        && script.version == *version
                        && checksum == reviewed_checksum
                }),
            "active dependency build script {} {} is outside the API audit's checksum-bound reviewed set",
            script.name,
            script.version
        );
    }
    Ok(())
}

fn parse_cargo_target(
    canonical_root: &Path,
    package: &str,
    package_id: &str,
    value: &crate::json::JsonValue,
) -> Result<CargoTarget> {
    let target = crate::json::json_object(value, "Cargo metadata target")?;
    let name = crate::json::json_string(
        crate::json::json_field(target, "name", "Cargo metadata target")?,
        "Cargo metadata target `name`",
    )?;
    let kinds = crate::json::json_string_array(
        crate::json::json_field(target, "kind", "Cargo metadata target")?,
        "Cargo metadata target `kind`",
    )?;
    let kind = CargoTargetKind::parse(&kinds)
        .with_context(|| format!("classifying target `{package}:{name}`"))?;
    let crate_types = crate::json::json_string_array(
        crate::json::json_field(target, "crate_types", "Cargo metadata target")?,
        "Cargo metadata target `crate_types`",
    )?;
    let executable = crate_types.contains(&"bin");
    let test = match crate::json::json_field(target, "test", "Cargo metadata target")? {
        crate::json::JsonValue::Bool(test) => *test,
        _ => bail!("Cargo metadata target `test` is not a Boolean"),
    };
    if let Some(required) = target.get("required-features") {
        let required =
            crate::json::json_string_array(required, "Cargo metadata target `required-features`")?;
        ensure!(
            required.is_empty(),
            "Cargo target `{package}:{name}` uses required-features, which the API audit does not yet model"
        );
    }
    let source = PathBuf::from(crate::json::json_string(
        crate::json::json_field(target, "src_path", "Cargo metadata target")?,
        "Cargo metadata target `src_path`",
    )?);
    let canonical_source = std::fs::canonicalize(&source)
        .with_context(|| format!("canonicalizing Cargo target source {}", source.display()))?;
    let relative = canonical_source
        .strip_prefix(canonical_root)
        .with_context(|| {
            format!(
                "Cargo target source {} is outside the API audit copy",
                source.display()
            )
        })?
        .to_path_buf();
    ensure!(
        relative
            .extension()
            .is_some_and(|extension| extension == "rs"),
        "Cargo target source {} is not Rust source",
        relative.display()
    );
    Ok(CargoTarget {
        package: package.to_string(),
        package_id: package_id.to_string(),
        name: name.to_string(),
        relative,
        kind,
        test,
        executable,
    })
}

pub(super) fn validate_workspace_build_script(
    root: &Path,
    package: &str,
    target: &CargoTarget,
) -> Result<()> {
    ensure!(
        package == crate::paths::KERNEL_PACKAGE && target.relative == Path::new("kernel/build.rs"),
        "active workspace build script `{package}:{}` is outside the API audit's exact build-script contract",
        target.relative.display()
    );
    let path = root.join(&target.relative);
    let source = read_bounded_authority_text(&path, "API audit build script")?;
    let syntax = syn::parse_file(&source).with_context(|| {
        format!(
            "parsing API audit build script {}",
            target.relative.display()
        )
    })?;
    ensure!(
        syntax.shebang.is_none()
            && syntax
                .attrs
                .iter()
                .all(|attribute| attribute.path().is_ident("doc")),
        "kernel build script has crate syntax outside its exact linker contract"
    );
    let [Item::Fn(main)] = syntax.items.as_slice() else {
        bail!("kernel build script must contain only its exact `main` function");
    };
    ensure!(
        main.attrs
            .iter()
            .all(|attribute| attribute.path().is_ident("doc"))
            && matches!(main.vis, Visibility::Inherited)
            && main.sig.ident == "main"
            && main.sig.constness.is_none()
            && main.sig.asyncness.is_none()
            && main.sig.unsafety.is_none()
            && main.sig.abi.is_none()
            && main.sig.generics.params.is_empty()
            && main.sig.generics.where_clause.is_none()
            && main.sig.inputs.is_empty()
            && main.sig.variadic.is_none()
            && matches!(main.sig.output, syn::ReturnType::Default),
        "kernel build script `main` has syntax outside its exact linker contract"
    );
    let actual = source_tokens(&source, main.block.span())?;
    let expected = <TokenStream as core::str::FromStr>::from_str(
        r#"{
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .expect("Cargo sets CARGO_MANIFEST_DIR for build scripts");
        println!("cargo:rustc-link-arg=-T{manifest_dir}/linker.ld");
        println!("cargo:rerun-if-changed=linker.ld");
    }"#,
    )
    .map_err(|_| anyhow::anyhow!("parsing the built-in kernel build-script contract"))?
    .to_string();
    ensure!(
        actual == expected,
        "kernel build script performs work outside its exact linker contract"
    );
    Ok(())
}

fn source_tokens(source: &str, span: Span) -> Result<String> {
    let start = byte_offset(source, span.start())?;
    let end = byte_offset(source, span.end())?;
    let selected = source
        .get(start..end)
        .context("source span is not an ordered UTF-8 range")?;
    Ok(<TokenStream as core::str::FromStr>::from_str(selected)
        .map_err(|_| anyhow::anyhow!("parsing source span as Rust tokens"))?
        .to_string())
}
