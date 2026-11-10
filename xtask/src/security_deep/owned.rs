use super::*;

#[derive(PartialEq, Eq, Debug)]
pub(super) struct TreeIdentity {
    pub(super) entries: u64,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

pub(super) fn scrub_verified_stale_credential_projection(root: &Path) -> Result<()> {
    require_scratch_cleanup_target(root)?;
    scrub_stale_credential_at_owned_root(root)
}

pub(super) fn scrub_stale_credential_at_owned_root(root: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root).with_context(|| {
        format!(
            "inspecting retained Codex Security scratch {}",
            root.display()
        )
    })?;
    ensure!(
        crate::mutate::workspace::metadata_is_real_directory(&metadata)
            && canonical_directory(root, "retained Codex Security scratch")? == root,
        "retained Codex Security scratch is redirected"
    );
    let home = root.join("codex-home");
    let home_metadata =
        std::fs::symlink_metadata(&home).context("inspecting retained private Codex home")?;
    ensure!(
        crate::mutate::workspace::metadata_is_real_directory(&home_metadata)
            && !metadata_is_reparse_point(&home_metadata)
            && canonical_directory(&home, "retained private Codex home")? == home,
        "retained private Codex home is redirected"
    );
    let auth = home.join("auth.json");
    match std::fs::symlink_metadata(&auth) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("inspecting retained ChatGPT credential"),
        Ok(metadata) => ensure!(
            metadata.file_type().is_file()
                && !metadata.file_type().is_symlink()
                && !metadata_is_reparse_point(&metadata),
            "retained ChatGPT credential is redirected or special"
        ),
    }
    std::fs::remove_file(&auth).context("unlinking retained ChatGPT credential name")?;
    require_absent(&auth, "retained ChatGPT credential name")?;
    Ok(())
}

pub(super) fn prepare_private_codex_home(scratch: &OwnedScratch) -> Result<PrivateCodexCredential> {
    let profile = std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?;
    let supplied = PathBuf::from(profile).join(".codex");
    let live = canonical_directory(&supplied, "ChatGPT Codex home")?;
    ensure!(
        canonical_spelling_matches(&live, &supplied),
        "ChatGPT Codex home is redirected"
    );
    prepare_private_codex_home_from(&live, scratch)
}

pub(super) fn canonical_spelling_matches(canonical: &Path, supplied: &Path) -> bool {
    crate::cargo::child_process_path(canonical) == crate::cargo::child_process_path(supplied)
}

pub(super) fn prepare_private_codex_home_from(
    live: &Path,
    scratch: &OwnedScratch,
) -> Result<PrivateCodexCredential> {
    let live = canonical_directory(live, "ChatGPT Codex home")?;
    let private = canonical_directory(&scratch.codex_home(), "private Codex home")?;
    let auth_path = live.join("auth.json");
    let auth_metadata = std::fs::symlink_metadata(&auth_path)
        .context("inspecting ChatGPT authentication artifact")?;
    ensure!(
        auth_metadata.file_type().is_file()
            && !auth_metadata.file_type().is_symlink()
            && !metadata_is_reparse_point(&auth_metadata)
            && auth_metadata.len() > 0
            && auth_metadata.len() <= MAX_CODEX_AUTH_BYTES,
        "ChatGPT authentication artifact is redirected, empty, or oversized"
    );
    let source = open_locked_auth(&auth_path)?;
    let locked_metadata = source.metadata().context("measuring ChatGPT credential")?;
    let identity = file_identity(&auth_path)?;
    ensure!(
        locked_metadata.len() == auth_metadata.len(),
        "ChatGPT authentication artifact changed while it was locked"
    );
    let source_links_before = identity.links;
    ensure!(
        source_links_before == 1,
        "ChatGPT authentication artifact has a pre-existing hard-link alias"
    );
    let mut credential =
        PrivateCodexCredential::new(scratch, auth_path, source, identity, source_links_before);
    std::fs::hard_link(&credential.source_path, &credential.auth_path)
        .context("projecting ChatGPT credential into the private Codex home")?;
    write_new_file(&private.join("config.toml"), b"")?;
    require_private_codex_home_seed(&private, &credential)?;
    credential.armed = true;
    Ok(credential)
}

pub(super) fn require_private_codex_home_seed(
    private: &Path,
    credential: &PrivateCodexCredential,
) -> Result<()> {
    let private = canonical_directory(private, "private Codex home")?;
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(&private).context("listing private Codex home")? {
        let entry = entry.context("reading private Codex home")?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("private Codex home entry is not UTF-8"))?;
        ensure!(names.insert(name), "private Codex home entry is duplicated");
        ensure!(
            names.len() <= 2,
            "private Codex home contains ambient state"
        );
    }
    ensure!(
        names == BTreeSet::from(["auth.json".to_string(), "config.toml".to_string()]),
        "private Codex home contains ambient state"
    );
    let linked =
        require_exact_credential_projection(&credential.source_path, &credential.auth_path)?;
    ensure!(
        credential.source_links_before == 1
            && linked.id == credential.identity.id
            && read(&private.join("config.toml"), 1, "private Codex config")?.is_empty(),
        "private Codex home seed differs from its exact hard-link policy"
    );
    require_no_ambient_codex_inputs(&private)
}

pub(super) fn require_no_ambient_codex_inputs(private: &Path) -> Result<()> {
    let private = canonical_directory(private, "private Codex home")?;
    ensure!(
        read(&private.join("config.toml"), 1, "private Codex config",)?.is_empty(),
        "private Codex config changed"
    );
    require_qualified_system_skills(&private.join("skills"))?;
    for name in ["rules", "plugins", "memories"] {
        require_absent(
            &private.join(name),
            &format!("private Codex ambient {name}"),
        )?;
    }
    Ok(())
}

fn require_qualified_system_skills(root: &Path) -> Result<()> {
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("inspecting private Codex system skills"),
        Ok(metadata) => ensure!(
            crate::mutate::workspace::metadata_is_real_directory(&metadata)
                && !metadata_is_reparse_point(&metadata)
                && canonical_directory(root, "private Codex system skills")? == root,
            "private Codex system skills root is redirected"
        ),
    }
    let identity = measure_qualified_system_skills(root)?;
    ensure!(
        codex_system_skills_identity_is_allowed(&identity),
        "private Codex system skills differ from the qualified runtime: observed {} entries, {} bytes, sha256 {}",
        identity.entries,
        identity.bytes,
        identity.sha256
    );
    Ok(())
}

pub(super) fn measure_qualified_system_skills(root: &Path) -> Result<TreeIdentity> {
    let entries = list_qualified_system_skill_entries(root)?;
    ensure!(
        entries.len() as u64 <= CODEX_SYSTEM_SKILLS_ENTRIES,
        "private Codex system skills exceed their entry limit"
    );
    let shapes: Vec<_> = entries
        .iter()
        .map(|(relative, _, directory)| (relative.clone(), *directory))
        .collect();
    let mut folded = sha256::Hasher::new();
    let mut bytes = 0u64;
    for (relative, path, directory) in &entries {
        folded.update(&[u8::from(*directory)])?;
        folded.update(&(relative.len() as u64).to_be_bytes())?;
        folded.update(relative.as_bytes())?;
        if !directory {
            let remaining = CODEX_SYSTEM_SKILLS_BYTES.saturating_sub(bytes);
            let (length, digest) = measure_file(path, remaining, "private Codex system skill")?;
            bytes = bytes
                .checked_add(length)
                .context("Codex system-skills byte count overflowed")?;
            ensure!(
                bytes <= CODEX_SYSTEM_SKILLS_BYTES,
                "private Codex system skills exceed their byte limit"
            );
            folded.update(&length.to_be_bytes())?;
            folded.update(&digest)?;
        }
    }
    let after = list_qualified_system_skill_entries(root)?;
    ensure!(
        after
            .iter()
            .map(|(relative, _, directory)| (relative.clone(), *directory))
            .eq(shapes),
        "private Codex system skills changed while they were measured"
    );
    Ok(TreeIdentity {
        entries: entries.len() as u64,
        bytes,
        sha256: sha256::hex(&folded.finish()),
    })
}

fn list_qualified_system_skill_entries(root: &Path) -> Result<Vec<(String, PathBuf, bool)>> {
    let mut entries = Vec::new();
    for child in bounded_sorted_directory_entries(
        root,
        usize::try_from(CODEX_SYSTEM_SKILLS_ENTRIES)
            .context("Codex system-skills entry limit does not fit usize")?,
    )? {
        collect_qualified_system_skill_entries(root, &child.path(), 0, &mut entries)?;
    }
    sort_qualified_system_skill_entries(&mut entries);
    Ok(entries)
}

fn sort_qualified_system_skill_entries(entries: &mut [(String, PathBuf, bool)]) {
    entries.sort_by(|left, right| left.0.cmp(&right.0));
}

fn collect_qualified_system_skill_entries(
    root: &Path,
    path: &Path,
    depth: usize,
    entries: &mut Vec<(String, PathBuf, bool)>,
) -> Result<()> {
    ensure!(
        depth <= MAX_CODEX_SYSTEM_SKILLS_DEPTH,
        "private Codex system-skills nesting is excessive"
    );
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspecting Codex system skill {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink() && !metadata_is_reparse_point(&metadata),
        "private Codex system skills contain a redirected entry"
    );
    let relative = path
        .strip_prefix(root)
        .context("private Codex system-skills entry escaped its root")?
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .context("private Codex system-skills entry is not UTF-8")?
        .join("/");
    ensure!(
        !relative.is_empty(),
        "private Codex system-skills entry has no name"
    );
    let directory = crate::mutate::workspace::metadata_is_real_directory(&metadata);
    ensure!(
        directory || metadata.file_type().is_file(),
        "private Codex system skills contain a special entry"
    );
    entries.push((relative, path.to_path_buf(), directory));
    ensure!(
        entries.len() as u64 <= CODEX_SYSTEM_SKILLS_ENTRIES,
        "private Codex system skills exceed their entry limit"
    );
    if directory {
        let remaining = usize::try_from(CODEX_SYSTEM_SKILLS_ENTRIES)
            .context("Codex system-skills entry limit does not fit usize")?
            .saturating_sub(entries.len());
        for child in bounded_sorted_directory_entries(path, remaining)? {
            collect_qualified_system_skill_entries(root, &child.path(), depth + 1, entries)?;
        }
    }
    Ok(())
}

pub(super) fn codex_system_skills_identity_is_allowed(identity: &TreeIdentity) -> bool {
    if identity.entries == CODEX_SYSTEM_SKILLS_ENTRIES
        && identity.bytes == CODEX_SYSTEM_SKILLS_BYTES
        && identity.sha256 == CODEX_SYSTEM_SKILLS_SHA256
    {
        return true;
    }
    #[cfg(test)]
    {
        identity.entries == 3
            && identity.bytes == 10
            && identity.sha256 == "25c6e9a0acf5e3017e5f3b94cdfda7604cd57d14196c5a4456182275e77eb69d"
    }
    #[cfg(not(test))]
    {
        false
    }
}

pub(super) fn require_private_credential_projection(private: &Path) -> Result<()> {
    let private = canonical_directory(private, "private Codex home")?;
    require_no_ambient_codex_inputs(&private)?;
    let profile = std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?;
    let supplied_live = PathBuf::from(profile).join(".codex");
    let live = canonical_directory(&supplied_live, "ChatGPT Codex home")?;
    ensure!(
        canonical_spelling_matches(&live, &supplied_live),
        "ChatGPT Codex home is redirected"
    );
    require_exact_credential_projection(&live.join("auth.json"), &private.join("auth.json"))?;
    Ok(())
}

pub(super) fn require_exact_credential_projection(
    source: &Path,
    projection: &Path,
) -> Result<FileIdentity> {
    let source = file_identity(source)?;
    let projection = file_identity(projection)?;
    ensure!(
        source.id == projection.id && source.links == 2 && projection.links == 2,
        "private ChatGPT credential is not the exact live hard-link projection"
    );
    Ok(source)
}

pub(super) fn plugin_root() -> Result<PathBuf> {
    let profile = std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?;
    canonical_directory(
        &PathBuf::from(profile)
            .join(".codex")
            .join("plugins")
            .join("cache")
            .join("openai-curated-remote")
            .join("codex-security")
            .join(PRODUCER_VERSION),
        "installed Codex Security plugin",
    )
}

pub(super) fn load_plugin_package() -> Result<PluginPackage> {
    let root = plugin_root()?;
    load_plugin_package_at(root)
}

pub(super) fn load_plugin_package_at(root: PathBuf) -> Result<PluginPackage> {
    let integrity_path = root.join("integrity.json");
    let integrity = read(
        &integrity_path,
        MAX_PLUGIN_MANIFEST_BYTES,
        "Codex Security integrity manifest",
    )?;
    ensure!(
        integrity.len() as u64 == PLUGIN_INTEGRITY_BYTES
            && sha256::hex(&sha256::digest(&integrity)) == PLUGIN_INTEGRITY_SHA256,
        "installed Codex Security integrity manifest differs from the qualified package"
    );
    let value = parse_json(&integrity, "Codex Security integrity manifest")?;
    let manifest = json_object(&value, "Codex Security integrity manifest")?;
    require_json_fields(
        manifest,
        &["files", "hash_algorithm", "schema_version"],
        "Codex Security integrity manifest",
    )?;
    ensure!(
        string(
            manifest,
            "hash_algorithm",
            "Codex Security integrity manifest"
        )? == "sha256"
            && json_u64(
                json_field(
                    manifest,
                    "schema_version",
                    "Codex Security integrity manifest"
                )?,
                "Codex Security integrity schema version"
            )? == 1,
        "unsupported Codex Security integrity manifest"
    );
    let listed = json_array(
        json_field(manifest, "files", "Codex Security integrity manifest")?,
        "Codex Security integrity files",
    )?;
    ensure!(
        listed.len() == PLUGIN_FILES,
        "Codex Security package file cardinality differs"
    );
    let mut files = Vec::with_capacity(listed.len());
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for value in listed {
        let entry = json_object(value, "Codex Security integrity entry")?;
        require_json_fields(entry, &["path", "sha256"], "Codex Security integrity entry")?;
        let slash_path = string(entry, "path", "Codex Security integrity entry")?;
        let relative = plugin_relative_path(slash_path)?;
        ensure!(
            names.insert(slash_path.to_string()),
            "Codex Security integrity path is duplicated"
        );
        let expected = string(entry, "sha256", "Codex Security integrity entry")?;
        require_sha256(expected, "Codex Security integrity digest")?;
        let bytes = read(
            &root.join(&relative),
            MAX_PLUGIN_FILE_BYTES,
            "Codex Security package file",
        )?;
        ensure!(
            sha256::hex(&sha256::digest(&bytes)) == expected,
            "Codex Security package file `{slash_path}` differs from integrity.json"
        );
        total = total
            .checked_add(bytes.len() as u64)
            .context("Codex Security package length overflowed")?;
        ensure!(
            total <= MAX_PLUGIN_TREE_BYTES,
            "Codex Security package exceeds its tree byte limit"
        );
        files.push(PluginFile {
            relative,
            slash_path: slash_path.to_string(),
            sha256: expected.to_string(),
        });
    }
    require_plugin_descriptor(&root)?;
    Ok(PluginPackage {
        root,
        integrity,
        files,
    })
}

pub(super) fn plugin_relative_path(value: &str) -> Result<PathBuf> {
    ensure!(
        !value.is_empty()
            && !value.contains('\\')
            && !value.starts_with('/')
            && !value.ends_with('/'),
        "Codex Security integrity path is not canonical"
    );
    let mut relative = PathBuf::new();
    for component in value.split('/') {
        ensure!(
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.contains(':'),
            "Codex Security integrity path has an unsafe component"
        );
        relative.push(component);
    }
    ensure!(
        relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "Codex Security integrity path is not relative"
    );
    Ok(relative)
}

fn require_plugin_descriptor(root: &Path) -> Result<()> {
    let bytes = read(
        &root.join(".codex-plugin").join("plugin.json"),
        MAX_PLUGIN_MANIFEST_BYTES,
        "Codex Security plugin descriptor",
    )?;
    let value = parse_json(&bytes, "Codex Security plugin descriptor")?;
    let descriptor = json_object(&value, "Codex Security plugin descriptor")?;
    ensure!(
        string(descriptor, "name", "Codex Security plugin descriptor")? == MCP_SERVER_NAME
            && string(descriptor, "version", "Codex Security plugin descriptor")?
                == PRODUCER_VERSION,
        "Codex Security plugin descriptor differs from the qualified package"
    );
    Ok(())
}

pub(super) fn require_sha256(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} is not lowercase SHA-256"
    );
    Ok(())
}

pub(super) fn copy_plugin(package: &PluginPackage, destination: &Path) -> Result<()> {
    write_new_file(&destination.join("integrity.json"), &package.integrity)?;
    for file in &package.files {
        let target = destination.join(&file.relative);
        prepare_file_parent(destination, &file.relative)?;
        let bytes = read(
            &package.root.join(&file.relative),
            MAX_PLUGIN_FILE_BYTES,
            "Codex Security package file",
        )?;
        ensure!(
            sha256::hex(&sha256::digest(&bytes)) == file.sha256,
            "Codex Security package file `{}` changed during mirroring",
            file.slash_path
        );
        write_new_file(&target, &bytes)?;
    }
    require_plugin_descriptor(destination)
}

fn prepare_file_parent(root: &Path, relative: &Path) -> Result<()> {
    let parent = relative
        .parent()
        .context("Codex Security package file has no parent")?;
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let std::path::Component::Normal(name) = component else {
            bail!("Codex Security package parent is not relative");
        };
        let next = current.join(name);
        current = crate::mutate::workspace::prepare_real_child(
            &current,
            &next,
            "Codex Security package directory",
        )?;
    }
    Ok(())
}
