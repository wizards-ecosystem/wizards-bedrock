use super::*;

#[derive(Debug)]
pub(super) struct AuditWorkspace {
    pub(super) _lock: File,
    pub(super) source_root: PathBuf,
    pub(super) root: PathBuf,
    pub(super) build: PathBuf,
    pub(super) cargo_home: PathBuf,
    pub(super) registry_source: PathBuf,
    pub(super) cargo_inputs: BTreeMap<PathBuf, [u8; 32]>,
    pub(super) execution_root: PathBuf,
    pub(super) source: BTreeMap<PathBuf, [u8; 32]>,
}

pub(super) fn prepare_audit_target(paths: &Paths) -> Result<PathBuf> {
    crate::mutate::workspace::prepare_real_directory_tree(
        paths.target_dir(),
        "API audit target directory",
    )
}

impl AuditWorkspace {
    pub(super) fn create(paths: &Paths) -> Result<Self> {
        let source_root = crate::mutate::workspace::canonical_existing_real_directory(
            paths.root(),
            "API audit source repository",
        )?;
        let target = prepare_audit_target(paths)?;
        let state = crate::mutate::workspace::prepare_real_child(
            &target,
            &target.join("api-audit"),
            "API audit state directory",
        )?;
        let lock = open_audit_lock(&state)?;

        let root = reset_audit_directory(&state, "workspace")?;
        let build = reset_audit_directory(&state, "build")?;
        let cargo = reset_audit_directory(&state, "cargo")?;
        for file in COPY_FILES {
            copy_file(&source_root.join(file), &root.join(file))?;
        }
        for directory in COPY_DIRECTORIES {
            copy_tree(&source_root.join(directory), &root.join(directory))?;
        }
        let cargo_home = crate::mutate::workspace::prepare_real_child(
            &cargo,
            &cargo.join("home"),
            "API audit private Cargo home",
        )?;
        let registry_source = crate::mutate::workspace::prepare_real_child(
            &cargo,
            &cargo.join("registry-source"),
            "API audit locked registry source",
        )?;
        let config = cargo_home.join("config.toml");
        let mut config_options = OpenOptions::new();
        OpenOptions::create_new(&mut config_options, true);
        OpenOptions::write(&mut config_options, true);
        let mut config_file = OpenOptions::open(&config_options, &config)
            .with_context(|| format!("creating {}", config.display()))?;
        std::io::Write::write_all(&mut config_file, b"\n")
            .with_context(|| format!("writing {}", config.display()))?;
        crate::mutate::workspace::require_direct_file(
            &cargo_home,
            &config,
            "API audit private Cargo configuration",
        )?;
        let execution_root = cargo_execution_root(&source_root)?;
        let compiler = compiler_identity(&source_root)?;
        let copied_source = source_snapshot(&root)?;
        materialize_locked_registry_source(
            &root,
            &cargo_home,
            &registry_source,
            &execution_root,
            &compiler,
        )?;
        ensure!(
            source_snapshot(&root)? == copied_source,
            "Cargo changed the API audit workspace while materializing registry archives"
        );
        let cargo_inputs = cargo_input_snapshot(&cargo, &cargo_home, &registry_source)?;
        let source = source_snapshot(&root)?;

        Ok(Self {
            _lock: lock,
            source_root,
            root,
            build,
            cargo_home,
            registry_source,
            cargo_inputs,
            execution_root,
            source,
        })
    }

    pub(super) fn require_source_unchanged(&self, source_root: &Path) -> Result<()> {
        let current = source_snapshot(source_root)?;
        if current == self.source {
            return Ok(());
        }
        let changed = self
            .source
            .keys()
            .chain(current.keys())
            .find(|path| self.source.get(*path) != current.get(*path))
            .context("the API audit source manifest changed without naming a file")?;
        bail!(
            "{} changed while the public API audit was running",
            changed.display()
        )
    }

    pub(super) fn require_copy_unchanged(
        &self,
        expected: &BTreeMap<PathBuf, [u8; 32]>,
    ) -> Result<()> {
        let current = source_snapshot(&self.root)?;
        if current == *expected {
            return Ok(());
        }
        let changed = expected
            .keys()
            .chain(current.keys())
            .find(|path| expected.get(*path) != current.get(*path))
            .context("the API audit copy changed without naming a file")?;
        bail!(
            "{} changed inside the API audit copy during compilation",
            changed.display()
        )
    }

    pub(super) fn require_cargo_inputs_unchanged(&self) -> Result<()> {
        let cargo = self
            .cargo_home
            .parent()
            .context("private Cargo home has no audit root")?;
        let current = cargo_input_snapshot(cargo, &self.cargo_home, &self.registry_source)?;
        ensure!(
            current == self.cargo_inputs,
            "the API audit's private Cargo configuration or locked registry source changed"
        );
        Ok(())
    }
}

pub(super) fn open_audit_lock(state: &Path) -> Result<File> {
    let lock_path = state.join("LOCK");
    crate::mutate::workspace::require_direct_file_if_present(state, &lock_path, "API audit lock")?;
    let mut lock_options = OpenOptions::new();
    OpenOptions::create(&mut lock_options, true);
    OpenOptions::truncate(&mut lock_options, false);
    OpenOptions::read(&mut lock_options, true);
    OpenOptions::write(&mut lock_options, true);
    let lock = OpenOptions::open(&lock_options, &lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    crate::mutate::workspace::require_direct_file(state, &lock_path, "API audit lock")?;
    lock.try_lock()
        .context("another public API audit is already running")?;
    Ok(lock)
}

pub(super) fn reset_audit_directory(state: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        matches!(name, "workspace" | "build" | "cargo"),
        "refusing to reset unknown API audit directory `{name}`"
    );
    let directory = state.join(name);
    ensure!(
        directory.parent() == Some(state),
        "API audit directory escaped its state root"
    );
    match std::fs::symlink_metadata(&directory) {
        Ok(_) => {
            let canonical = crate::mutate::workspace::canonical_existing_real_directory(
                &directory,
                "API audit working directory",
            )?;
            ensure!(
                canonical.parent() == Some(state),
                "API audit working directory escaped its canonical state root"
            );
            std::fs::remove_dir_all(&canonical)
                .with_context(|| format!("removing stale {}", canonical.display()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", directory.display()));
        }
    }
    crate::mutate::workspace::prepare_real_child(state, &directory, "API audit working directory")
}

pub(super) fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    crate::mutate::workspace::canonical_existing_real_directory(
        source,
        "API audit source directory",
    )?;
    let parent = destination
        .parent()
        .context("API audit copy directory has no parent")?;
    crate::mutate::workspace::prepare_real_child(parent, destination, "API audit copy directory")?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("reading an entry in {}", source.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            copy_file(&entry.path(), &target)?;
        } else {
            bail!(
                "API audit source {} is not a regular file",
                entry.path().display()
            );
        }
    }
    Ok(())
}

pub(super) fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    let source_parent = source
        .parent()
        .context("API audit source file has no parent")?;
    crate::mutate::workspace::canonical_existing_real_directory(
        source_parent,
        "API audit source-file parent",
    )?;
    crate::mutate::workspace::require_direct_file(source_parent, source, "API audit source file")?;
    let parent = destination
        .parent()
        .context("API audit copied file has no parent")?;
    crate::mutate::workspace::canonical_existing_real_directory(
        parent,
        "API audit copied-file parent",
    )?;
    let mut input = crate::mutate::workspace::open_real_file(source, "API audit source file")?;
    let mut output_options = OpenOptions::new();
    OpenOptions::create_new(&mut output_options, true);
    OpenOptions::write(&mut output_options, true);
    let mut output = OpenOptions::open(&output_options, destination)
        .with_context(|| format!("creating API audit copy {}", destination.display()))?;
    std::io::copy(&mut input, &mut output).with_context(|| {
        format!(
            "copying API audit source {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    std::io::Write::flush(&mut output)
        .with_context(|| format!("flushing API audit copy {}", destination.display()))?;
    crate::mutate::workspace::require_direct_file(parent, destination, "API audit copied file")?;
    Ok(())
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) struct LockedRegistryPackage {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) checksum: String,
}

pub(super) fn locked_registry_packages(lock: &Path) -> Result<Vec<LockedRegistryPackage>> {
    let text = read_bounded_authority_text(lock, "API audit Cargo.lock")?;
    ensure!(
        text.lines().any(|line| line == "version = 4"),
        "API audit requires Cargo.lock format 4"
    );
    let mut packages = Vec::new();
    for block in text.split("[[package]]").skip(1) {
        let mut name = None;
        let mut version = None;
        let mut source = None;
        let mut checksum = None;
        for line in block.lines().map(str::trim) {
            let value = |key: &str| {
                line.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix(" = \""))
                    .and_then(|rest| rest.strip_suffix('"'))
            };
            if let Some(value) = value("name") {
                name = Some(value.to_string());
            } else if let Some(value) = value("version") {
                version = Some(value.to_string());
            } else if let Some(value) = value("source") {
                source = Some(value.to_string());
            } else if let Some(value) = value("checksum") {
                checksum = Some(value.to_string());
            }
        }
        let Some(source) = source else {
            continue;
        };
        ensure!(
            source == "registry+https://github.com/rust-lang/crates.io-index",
            "API audit has no private source contract for `{source}`"
        );
        let name = name.context("locked registry package has no name")?;
        let version = version.context("locked registry package has no version")?;
        let checksum = checksum.context("locked registry package has no checksum")?;
        for (label, value) in [("name", &name), ("version", &version)] {
            ensure!(
                !value.is_empty()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+')
                    }),
                "locked registry package has unsafe {label} `{value}`"
            );
        }
        ensure!(
            checksum.len() == 64
                && checksum
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "locked registry package {name} {version} has malformed checksum"
        );
        packages.push(LockedRegistryPackage {
            name,
            version,
            checksum,
        });
    }
    packages.sort();
    packages.dedup();
    ensure!(!packages.is_empty(), "Cargo.lock has no registry packages");
    Ok(packages)
}

fn ambient_cargo_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .context("the API audit cannot locate Cargo's registry source cache")?;
    Ok(PathBuf::from(home).join(".cargo"))
}

pub(super) fn materialize_locked_registry_source(
    workspace: &Path,
    cargo_home: &Path,
    destination: &Path,
    execution_root: &Path,
    compiler: &CompilerIdentity,
) -> Result<()> {
    let packages = locked_registry_packages(&workspace.join("Cargo.lock"))?;
    let ambient = ambient_cargo_home()?.join("registry");
    let caches = registry_roots(&ambient.join("cache"), "Cargo registry archive cache")?;
    for package in &packages {
        let archive_name = format!("{}-{}.crate", package.name, package.version);
        let matches = caches
            .iter()
            .map(|root| root.join(&archive_name))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        let [archive] = matches.as_slice() else {
            bail!(
                "locked registry package {} {} has {} cached archives",
                package.name,
                package.version,
                matches.len()
            );
        };
        let digest = stream_file_digest(archive, "API audit registry archive")?;
        ensure!(
            crate::sha256::hex(&digest) == package.checksum,
            "cached registry archive {} does not match Cargo.lock",
            archive.display()
        );
        let cache_name = archive
            .parent()
            .and_then(Path::file_name)
            .context("registry archive cache has no source identity")?;
        let private_cache = prepare_relative_directory(
            cargo_home,
            Path::new("registry").join("cache").join(cache_name),
            "API audit private registry archive cache",
        )?;
        copy_file(archive, &private_cache.join(&archive_name))?;

        let index = ambient.join("index").join(cache_name);
        let private_index = prepare_relative_directory(
            cargo_home,
            Path::new("registry").join("index").join(cache_name),
            "API audit private registry index",
        )?;
        if !private_index.join("config.json").exists() {
            copy_file(
                &index.join("config.json"),
                &private_index.join("config.json"),
            )?;
        }
        let entry = registry_index_entry(&package.name);
        let private_parent = prepare_relative_directory(
            &private_index,
            Path::new(".cache").join(entry.parent().unwrap_or(Path::new(""))),
            "API audit private registry index entry parent",
        )?;
        let private_entry = private_parent.join(
            entry
                .file_name()
                .context("registry index entry has no name")?,
        );
        if !private_entry.exists() {
            copy_file(&index.join(".cache").join(&entry), &private_entry)?;
        }
    }

    let mut command = base_cargo_command(execution_root, cargo_home, compiler);
    command
        .args(["vendor", "--offline", "--locked", "--versioned-dirs"])
        .arg(destination)
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"));
    let captured = crate::bounded_process::output_with_limit(
        command,
        REGISTRY_MATERIALIZATION_TIMEOUT,
        "API audit registry materialization",
        MAX_AUDIT_OUTPUT_BYTES,
    )
    .context("materializing checksum-verified registry archives for the API audit")?;
    ensure!(
        captured.status.success(),
        "materializing the API audit registry source failed ({}):\n{}{}",
        captured.status,
        String::from_utf8_lossy(&captured.stdout),
        String::from_utf8_lossy(&captured.stderr)
    );
    compiler.require_unchanged()?;
    validate_materialized_registry_source(destination, &packages)?;
    reset_materialization_cargo_home(cargo_home)?;
    Ok(())
}

fn reset_materialization_cargo_home(cargo_home: &Path) -> Result<()> {
    let parent = cargo_home
        .parent()
        .context("private Cargo home has no audit parent")?;
    let canonical_parent = crate::mutate::workspace::canonical_existing_real_directory(
        parent,
        "API audit Cargo-state root",
    )?;
    let canonical_home = crate::mutate::workspace::canonical_existing_real_directory(
        cargo_home,
        "API audit materialization Cargo home",
    )?;
    ensure!(
        canonical_home.parent() == Some(canonical_parent.as_path()),
        "private Cargo home escaped its audit parent"
    );
    std::fs::remove_dir_all(&canonical_home)
        .with_context(|| format!("removing bootstrap Cargo home {}", canonical_home.display()))?;
    let cargo_home = crate::mutate::workspace::prepare_real_child(
        &canonical_parent,
        &canonical_parent.join("home"),
        "API audit private Cargo home",
    )?;
    let config = cargo_home.join("config.toml");
    std::fs::write(&config, b"\n").with_context(|| format!("writing {}", config.display()))?;
    crate::mutate::workspace::require_direct_file(
        &cargo_home,
        &config,
        "API audit private Cargo configuration",
    )?;
    Ok(())
}

fn registry_roots(root: &Path, label: &str) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry.with_context(|| format!("reading an entry in {}", root.display()))?;
        if entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?
            .is_dir()
        {
            roots.push(entry.path());
        }
    }
    ensure!(!roots.is_empty(), "{label} is empty at {}", root.display());
    Ok(roots)
}

fn registry_index_entry(name: &str) -> PathBuf {
    match name.len() {
        1 => PathBuf::from("1").join(name),
        2 => PathBuf::from("2").join(name),
        3 => PathBuf::from("3").join(&name[..1]).join(name),
        _ => PathBuf::from(&name[..2]).join(&name[2..4]).join(name),
    }
}

fn prepare_relative_directory(root: &Path, relative: PathBuf, label: &str) -> Result<PathBuf> {
    let mut parent = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            bail!("{label} has unsafe relative path {}", relative.display());
        };
        let child = parent.join(name);
        parent = crate::mutate::workspace::prepare_real_child(&parent, &child, label)?;
    }
    Ok(parent)
}

fn validate_materialized_registry_source(
    source: &Path,
    packages: &[LockedRegistryPackage],
) -> Result<()> {
    let expected = packages
        .iter()
        .map(|package| format!("{}-{}", package.name, package.version))
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("reading materialized registry source {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("reading an entry in {}", source.display()))?;
        ensure!(
            entry
                .file_type()
                .with_context(|| format!("reading the type of {}", entry.path().display()))?
                .is_dir(),
            "materialized registry source {} is not a directory",
            entry.path().display()
        );
        actual.insert(entry.file_name().to_string_lossy().into_owned());
    }
    ensure!(
        actual == expected,
        "materialized registry source differs from the exact Cargo.lock package set"
    );
    for package in packages {
        validate_materialized_registry_package(
            &source.join(format!("{}-{}", package.name, package.version)),
            package,
        )?;
    }
    Ok(())
}

pub(super) fn validate_materialized_registry_package(
    directory: &Path,
    package: &LockedRegistryPackage,
) -> Result<()> {
    let checksum_path = directory.join(".cargo-checksum.json");
    let checksum_bytes = read_bounded_authority_file(
        &checksum_path,
        "materialized registry package checksum metadata",
    )?;
    let checksum_json = crate::json::parse_json(
        &checksum_bytes,
        "materialized registry package checksum metadata",
    )?;
    let checksum_object = crate::json::json_object(
        &checksum_json,
        "materialized registry package checksum metadata",
    )?;
    crate::json::require_json_fields(
        checksum_object,
        &["$comment", "files", "package"],
        "materialized registry package checksum metadata",
    )?;
    let _comment = crate::json::json_string(
        crate::json::json_field(
            checksum_object,
            "$comment",
            "materialized registry package checksum metadata",
        )?,
        "materialized registry package checksum comment",
    )?;
    let recorded = crate::json::json_string(
        crate::json::json_field(
            checksum_object,
            "package",
            "materialized registry package checksum metadata",
        )?,
        "materialized registry package checksum",
    )?;
    ensure!(
        recorded == package.checksum,
        "materialized registry package {} {} does not match Cargo.lock",
        package.name,
        package.version
    );
    let files = crate::json::json_object(
        crate::json::json_field(
            checksum_object,
            "files",
            "materialized registry package checksum metadata",
        )?,
        "materialized registry package file checksums",
    )?;
    let mut materialized = BTreeSet::new();
    collect_materialized_files(directory, directory, &mut materialized)?;
    materialized.remove(".cargo-checksum.json");
    let recorded_files = files.keys().cloned().collect::<BTreeSet<_>>();
    ensure!(
        materialized == recorded_files,
        "materialized registry package {} {} file set differs from its authenticated archive",
        package.name,
        package.version
    );
    for (relative, digest) in files {
        ensure!(
            !relative.contains('\\')
                && Path::new(relative)
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))),
            "materialized registry package has unsafe checksum path `{relative}`"
        );
        let digest = crate::json::json_string(digest, "materialized registry file checksum")?;
        let path = directory.join(relative);
        let actual = stream_marker_free_digest(&path, "authenticated registry compiler input")?;
        ensure!(
            crate::sha256::hex(&actual) == digest,
            "materialized registry file `{relative}` differs from its authenticated archive"
        );
    }
    Ok(())
}

fn collect_materialized_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            collect_materialized_files(root, &entry.path(), files)?;
        } else if kind.is_file() {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .with_context(|| {
                    format!(
                        "making {} relative to materialized registry root {}",
                        path.display(),
                        root.display()
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/");
            ensure!(
                files.insert(relative.clone()),
                "duplicate registry file `{relative}`"
            );
        } else {
            bail!(
                "materialized registry source {} is not a regular file",
                entry.path().display()
            );
        }
    }
    Ok(())
}

pub(super) fn cargo_input_snapshot(
    cargo_root: &Path,
    cargo_home: &Path,
    registry_source: &Path,
) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutate::workspace::canonical_existing_real_directory(
        cargo_root,
        "API audit Cargo-state root",
    )?;
    crate::mutate::workspace::canonical_existing_real_directory(
        cargo_home,
        "API audit private Cargo home",
    )?;
    crate::mutate::workspace::canonical_existing_real_directory(
        registry_source,
        "API audit private registry source",
    )?;
    let mut snapshot = BTreeMap::new();
    snapshot_file(cargo_root, &cargo_home.join("config.toml"), &mut snapshot)?;
    snapshot_tree(cargo_root, registry_source, &mut snapshot)?;
    Ok(snapshot)
}

pub(super) fn cargo_execution_root(repository: &Path) -> Result<PathBuf> {
    let repository = std::fs::canonicalize(repository)
        .with_context(|| format!("canonicalizing {}", repository.display()))?;
    let execution_root = repository
        .ancestors()
        .last()
        .context("the API audit repository has no filesystem root")?;
    let config = execution_root.join(".cargo");
    for name in ["config", "config.toml"] {
        let path = config.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => bail!(
                "the API audit refuses Cargo configuration at filesystem root {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", path.display()));
            }
        }
    }
    crate::mutate::workspace::canonical_existing_real_directory(
        execution_root,
        "API audit Cargo execution root",
    )
}

pub(super) fn source_snapshot(root: &Path) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutate::workspace::canonical_existing_real_directory(root, "API audit snapshot root")?;
    let mut snapshot = BTreeMap::new();
    for relative in COPY_FILES {
        snapshot_file(root, &root.join(relative), &mut snapshot)?;
    }
    for relative in COPY_DIRECTORIES {
        snapshot_tree(root, &root.join(relative), &mut snapshot)?;
    }
    Ok(snapshot)
}

fn snapshot_tree(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    crate::mutate::workspace::canonical_existing_real_directory(
        directory,
        "API audit snapshot directory",
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
            snapshot_tree(root, &entry.path(), snapshot)?;
        } else if kind.is_file() {
            snapshot_file(root, &entry.path(), snapshot)?;
        } else {
            bail!(
                "API audit source {} is not a regular file",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn snapshot_file(
    root: &Path,
    path: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    let parent = path.parent().context("snapshot file has no parent")?;
    crate::mutate::workspace::require_direct_file(parent, path, "API audit snapshot file")?;
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside the source snapshot", path.display()))?
        .to_path_buf();
    let digest = stream_file_digest(path, "API audit snapshot file")?;
    ensure!(
        snapshot.insert(relative.clone(), digest).is_none(),
        "{} occurs twice in the source snapshot",
        relative.display()
    );
    Ok(())
}

pub(super) fn snapshot_marker_free_file(
    root: &Path,
    path: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
    label: &str,
) -> Result<()> {
    let parent = path.parent().context("snapshot file has no parent")?;
    crate::mutate::workspace::require_direct_file(parent, path, label)?;
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside the {label} snapshot", path.display()))?
        .to_path_buf();
    let digest = stream_marker_free_digest(path, label)?;
    ensure!(
        snapshot.insert(relative.clone(), digest).is_none(),
        "{} occurs twice in the {label} snapshot",
        relative.display()
    );
    Ok(())
}

pub(super) fn stream_marker_free_digest(path: &Path, label: &str) -> Result<[u8; 32]> {
    stream_marker_free_digest_with_limit(path, label, MAX_COMPILER_AUTHORITY_TEXT_BYTES)
}

pub(super) fn stream_marker_free_digest_with_limit(
    path: &Path,
    label: &str,
    maximum_text_bytes: usize,
) -> Result<[u8; 32]> {
    let mut file = crate::mutate::workspace::open_real_file(path, label)?;
    let mut hasher = crate::sha256::Hasher::new();
    let mut buffer = [0u8; COMPILER_AUTHORITY_BUFFER_BYTES];
    let mut marker_window = Vec::with_capacity(COMPILER_AUTHORITY_BUFFER_BYTES + MARKER.len());
    let mut utf8_window = Vec::with_capacity(COMPILER_AUTHORITY_BUFFER_BYTES + 3);
    let mut text = Vec::new();
    let mut valid_utf8 = true;
    let mut text_oversized = false;

    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("reading {}", path.display()))?;
        if read == 0 {
            break;
        }
        let bytes = &buffer[..read];
        hasher
            .update(bytes)
            .with_context(|| format!("hashing {}", path.display()))?;

        marker_window.extend_from_slice(bytes);
        ensure!(
            !bytes_contain_diagnostic_marker(&marker_window),
            "{label} {} contains the API audit's reserved diagnostic marker",
            path.display()
        );
        let retained = MARKER.len().min(marker_window.len());
        let retained_start = marker_window.len() - retained;
        marker_window.copy_within(retained_start.., 0);
        marker_window.truncate(retained);

        if valid_utf8 {
            utf8_window.extend_from_slice(bytes);
            match std::str::from_utf8(&utf8_window) {
                Ok(_) => utf8_window.clear(),
                Err(error) if error.error_len().is_none() => {
                    let valid = error.valid_up_to();
                    utf8_window.drain(..valid);
                    ensure!(
                        utf8_window.len() <= 3,
                        "UTF-8 validation retained more than one incomplete scalar"
                    );
                }
                Err(_) => {
                    valid_utf8 = false;
                    utf8_window.clear();
                    text.clear();
                }
            }
            if valid_utf8 && !text_oversized {
                let next = text
                    .len()
                    .checked_add(bytes.len())
                    .context("compiler authority text length overflowed")?;
                if next <= maximum_text_bytes {
                    text.extend_from_slice(bytes);
                } else {
                    text_oversized = true;
                    text.clear();
                }
            }
        }
    }

    if valid_utf8 && !utf8_window.is_empty() {
        valid_utf8 = false;
        text.clear();
    }
    if valid_utf8 {
        ensure!(
            !text_oversized,
            "{label} {} is valid UTF-8 beyond its {maximum_text_bytes}-byte semantic scan ceiling",
            path.display()
        );
        reject_authority_diagnostic_marker(path, &text, label)?;
    } else {
        ensure!(
            path.extension().is_none_or(|extension| extension != "rs"),
            "Rust compiler input {} is not UTF-8",
            path.display()
        );
    }
    Ok(hasher.finish())
}

pub(super) fn snapshot_marker_free_tree(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
    label: &str,
) -> Result<()> {
    crate::mutate::workspace::canonical_existing_real_directory(directory, label)?;
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            snapshot_marker_free_tree(root, &entry.path(), snapshot, label)?;
        } else if kind.is_file() {
            snapshot_marker_free_file(root, &entry.path(), snapshot, label)?;
        } else {
            bail!("{label} {} is not a regular file", entry.path().display());
        }
    }
    Ok(())
}

pub(super) fn rejected_audit_environment_variable(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    if name.starts_with("CARGO_") {
        return !matches!(
            name.as_str(),
            "CARGO_BIN_NAME"
                | "CARGO_CRATE_NAME"
                | "CARGO_HOME"
                | "CARGO_LOG"
                | "CARGO_MAKEFLAGS"
                | "CARGO_MANIFEST_DIR"
                | "CARGO_MANIFEST_PATH"
                | "CARGO_PRIMARY_PACKAGE"
                | "CARGO_TARGET_DIR"
        ) && !name.starts_with("CARGO_PKG_")
            && !name.starts_with("CARGO_TERM_");
    }
    matches!(
        name.as_str(),
        "RUSTC"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
            | "RUSTC_BOOTSTRAP"
            | "RUSTFLAGS"
            | "RUSTDOC"
            | "RUSTDOCFLAGS"
            | "RUST_TARGET_PATH"
    ) || name.starts_with("RUSTUP_")
        && !matches!(
            name.as_str(),
            "RUSTUP_HOME" | "RUSTUP_TOOLCHAIN" | "RUSTUP_TOOLCHAIN_SOURCE"
        )
}

pub(super) fn remove_ambient_cargo_semantics(
    command: &mut Command,
    names: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for name in names {
        let normalized = name.to_string_lossy().to_ascii_uppercase();
        if normalized.starts_with("CARGO_")
            || normalized.starts_with("RUST")
            || normalized == "TARGET"
            || AMBIENT_COMPILER_PREFIXES
                .iter()
                .any(|prefix| normalized.starts_with(prefix))
        {
            command.env_remove(name);
        }
    }
}

pub(super) fn validate_api_audit_environment() -> Result<()> {
    let mut rejected = Vec::new();
    for (name, _) in std::env::vars_os() {
        let name = name.into_string().map_err(|_| {
            anyhow::anyhow!("the API audit environment has a non-UTF-8 variable name")
        })?;
        if rejected_audit_environment_variable(&name) {
            rejected.push(name);
        }
    }
    rejected.sort();
    ensure!(
        rejected.is_empty(),
        "the API audit refuses compiler-semantic environment variable(s): {}",
        rejected.join(", ")
    );
    Ok(())
}
