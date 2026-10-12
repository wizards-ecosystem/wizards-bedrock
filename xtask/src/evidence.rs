//! Reproducible release evidence and its offline verifier.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{Context, Result, ensure};

use crate::cargo::Profile;
use crate::paths::Paths;
use crate::{bounded_process, gate, image, mutate, qemu, qualification, sha256, tasks, vendor};

const FORMAT: &str = "wizard-evidence-v9";
const MANIFEST_NAME: &str = "MANIFEST";
const MANIFEST_TEMP: &str = "MANIFEST.incomplete";
const PASS: &str = "pass";
const AVAILABLE: &str = "available";
const UNAVAILABLE: &str = "unavailable";
const QUALIFICATION_TOOLS: &[&str] = &["tool.java", "tool.vmware"];
const IDENTITY_DEADLINE: Duration = Duration::from_secs(5);
const QEMU_VERSION_DEADLINE: Duration = Duration::from_secs(5);
const QEMU_VERSION_LABEL: &str = "QEMU version identity probe";
const MAX_GIT_IDENTITY_BYTES: usize = 4096;
const MAX_GIT_STATUS_BYTES: usize = 1024 * 1024;
const MAX_GIT_DIFF_BYTES: usize = 1024 * 1024;
const MAX_TOOL_VERSION_BYTES: usize = 64 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_SOURCE_LOCK_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TOOLCHAIN_FILE_BYTES: u64 = 64 * 1024;
const MAX_TOOL_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FIRMWARE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_KERNEL_BYTES: u64 = 64 * 1024 * 1024;
const ALTERNATE_BUILD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const ALTERNATE_BUILD_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MIRROR_ENTRY_LIMIT: usize = 100_000;
const MIRROR_DEPTH_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Digest {
    bytes: u64,
    sha256: String,
}

struct ReproducedRelease {
    image: PathBuf,
    kernel: PathBuf,
    host_roots: [PathBuf; 3],
}

#[derive(Debug, Clone, Copy)]
struct ArtifactSpec {
    prefix: &'static str,
    relative: &'static str,
}

const ARTIFACTS: &[ArtifactSpec] = &[
    ArtifactSpec {
        prefix: "artifact.path-a.kernel",
        relative: "path-a/wizard-kernel",
    },
    ArtifactSpec {
        prefix: "artifact.path-a.image",
        relative: "path-a/bedrock.img",
    },
    ArtifactSpec {
        prefix: "artifact.path-b.kernel",
        relative: "path-b/wizard-kernel",
    },
    ArtifactSpec {
        prefix: "artifact.path-b.image",
        relative: "path-b/bedrock.img",
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Record {
    fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QualificationRelation {
    qualified: qualification::SourceIdentity,
    policy: &'static str,
    files: Vec<String>,
    digest: String,
}

impl QualificationRelation {
    fn discover(paths: &Paths, qualified: &qualification::SourceIdentity) -> Result<Self> {
        let (commit, tree) = clean_source(paths.root())?;
        let evidence = qualification::SourceIdentity { commit, tree };
        if evidence == *qualified {
            return Ok(Self {
                qualified: qualified.clone(),
                policy: "same-commit",
                files: Vec::new(),
                digest: sha256::hex(&sha256::digest(&[])),
            });
        }

        let parents = git_output(
            paths.root(),
            &["rev-list", "--parents", "-n", "1", "HEAD"],
            MAX_GIT_IDENTITY_BYTES,
            "evidence source parents",
        )?;
        require_single_direct_parent(&parents, &evidence.commit, &qualified.commit)?;
        let parent_tree_selector = format!("{}^{{tree}}", qualified.commit);
        let parent_tree = git_output(
            paths.root(),
            &["rev-parse", &parent_tree_selector],
            MAX_GIT_IDENTITY_BYTES,
            "qualification parent tree",
        )?;
        ensure!(
            parent_tree == qualified.tree,
            "qualification parent tree differs from its records"
        );
        let raw = git_output_bytes(
            paths.root(),
            &[
                "diff-tree",
                "--no-commit-id",
                "--raw",
                "-r",
                "--no-renames",
                "-z",
                &qualified.commit,
                &evidence.commit,
                "--",
            ],
            MAX_GIT_DIFF_BYTES,
            "parent documentation diff",
        )?;
        Self::from_parent_diff(qualified, &raw)
    }

    fn from_parent_diff(qualified: &qualification::SourceIdentity, raw: &[u8]) -> Result<Self> {
        Ok(Self {
            qualified: qualified.clone(),
            policy: "parent-docs-only-v1",
            files: parse_parent_docs_diff(raw)?,
            digest: sha256::hex(&sha256::digest(raw)),
        })
    }

    fn add_to(&self, record: &mut Record) -> Result<()> {
        record.insert("qualification.source.commit", &self.qualified.commit)?;
        record.insert("qualification.source.tree", &self.qualified.tree)?;
        record.insert("qualification.relation", self.policy)?;
        record.insert("qualification.closure.files", self.files.len().to_string())?;
        record.insert("qualification.closure.paths", self.files.join(","))?;
        record.insert("qualification.closure.sha256", &self.digest)
    }
}

fn require_single_direct_parent(parents: &str, evidence: &str, qualified: &str) -> Result<()> {
    let words: Vec<_> = parents.split_whitespace().collect();
    ensure!(
        words == [evidence, qualified],
        "parent-docs-only-v1 requires the qualification source to be the single direct parent"
    );
    Ok(())
}

impl Record {
    fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) -> Result<()> {
        let key = key.into();
        let value = value.into();
        validate_field(&key, &value)?;
        ensure!(
            self.fields.insert(key.clone(), value).is_none(),
            "duplicate evidence key `{key}`"
        );
        Ok(())
    }

    fn get(&self, key: &str) -> Result<&str> {
        self.fields
            .get(key)
            .map(String::as_str)
            .with_context(|| format!("evidence manifest has no `{key}`"))
    }

    fn render(&self) -> String {
        let mut rendered = format!("{FORMAT}\n");
        for (key, value) in &self.fields {
            rendered.push_str(key);
            rendered.push('=');
            rendered.push_str(value);
            rendered.push('\n');
        }
        rendered
    }
}

/// Run the complete gate, reproduce the shipped artifacts twice and record the
/// exact source, tools, inputs, bytes and verdicts involved.
pub(crate) fn create(paths: &Paths) -> Result<()> {
    let root = invalidate_manifest(paths)?;
    println!("[xtask] qualifying a reproducible release from a clean checkout");
    qemu::nvme_image(paths.root()).context("materialising the deterministic NVMe input")?;
    vendor::verify(paths)?;
    let qualified_source = qualification::recorded_source(paths)?;
    let relation = QualificationRelation::discover(paths, &qualified_source)?;
    let mut before = input_record(paths)?;
    relation.add_to(&mut before)?;
    let registry = mutate::registry_fingerprint();
    let qualifications = qualification::snapshot(
        paths,
        &registry,
        mutate::MUTATIONS.len(),
        &qualified_source,
        relation_requires_current_mutation(relation.policy),
    )?;

    tasks::ci(paths).context("release evidence stopped at the local gate")?;
    ensure!(
        input_record_with_relation(paths, &qualified_source)? == before,
        "source, tools, firmware or boot inputs changed during qualification"
    );

    for child in [
        "work-a",
        "work-b",
        "path-a",
        "path-b",
        "qualification",
        "toolchain-b",
        "cargo-home-b",
    ] {
        reset_child(&root, child)?;
    }

    let release_a = reproduce_release(paths, &root.join("work-a"), "A")?;
    let release_b = reproduce_release_alternate(paths, &root, &root.join("work-b"))?;
    ensure!(
        release_a
            .host_roots
            .iter()
            .zip(&release_b.host_roots)
            .all(|(left, right)| left != right),
        "release reproductions reused a source, toolchain or Cargo root"
    );
    let original_roots = crate::cargo::active_kernel_remap_roots(paths)?;
    let host_roots = original_roots
        .iter()
        .chain(release_a.host_roots.iter())
        .chain(release_b.host_roots.iter())
        .cloned()
        .collect::<Vec<_>>();
    for (path, maximum, label) in [
        (&release_a.kernel, MAX_KERNEL_BYTES, "release kernel A"),
        (&release_a.image, boot_image_bytes(), "release image A"),
        (&release_b.kernel, MAX_KERNEL_BYTES, "release kernel B"),
        (&release_b.image, boot_image_bytes(), "release image B"),
    ] {
        require_no_host_paths(path, maximum, label, &host_roots)?;
    }

    let built_kernel_a =
        digest_file_bounded(&release_a.kernel, MAX_KERNEL_BYTES, "release kernel A")?;
    let built_kernel_b =
        digest_file_bounded(&release_b.kernel, MAX_KERNEL_BYTES, "release kernel B")?;
    let built_image_a = digest_file_exact(&release_a.image, boot_image_bytes(), "release image A")?;
    let built_image_b = digest_file_exact(&release_b.image, boot_image_bytes(), "release image B")?;
    ensure!(
        byte_identical(&built_kernel_a, &built_kernel_b),
        "clean release paths produced different kernel ELFs"
    );
    ensure!(
        byte_identical(&built_image_a, &built_image_b),
        "clean release paths produced different disk images"
    );

    let bundled_kernel_a = root.join(ARTIFACTS[0].relative);
    let bundled_image_a = root.join(ARTIFACTS[1].relative);
    let bundled_kernel_b = root.join(ARTIFACTS[2].relative);
    let bundled_image_b = root.join(ARTIFACTS[3].relative);
    copy_exact(&release_a.kernel, &bundled_kernel_a, &built_kernel_a)?;
    copy_exact(&release_a.image, &bundled_image_a, &built_image_a)?;
    copy_exact(&release_b.kernel, &bundled_kernel_b, &built_kernel_b)?;
    copy_exact(&release_b.image, &bundled_image_b, &built_image_b)?;

    tasks::boot_shipped_image(paths, &bundled_image_a)
        .context("booting the exact path-A evidence image")?;
    ensure!(
        digest_file_exact(&bundled_image_a, boot_image_bytes(), "booted release image",)?
            == built_image_a,
        "the boot changed the release image it qualified"
    );

    remove_child(&root, "work-a")?;
    remove_child(&root, "work-b")?;

    let mut record = input_record_with_relation(paths, &qualified_source)?;
    ensure!(
        record == before,
        "source, tools, firmware or boot inputs changed during artifact reproduction"
    );
    qualifications.install(&root)?;
    add_artifacts(&mut record, &root)?;
    add_qualification_files(&mut record, &root)?;
    verify_qualification_artifact_identity(paths, &root)?;
    for step in gate::REGISTRY.steps() {
        record.insert(step.evidence_key(), PASS)?;
    }
    record.insert("result.artifacts-byte-identical", PASS)?;
    record.insert("result.shipped-artifact-boot", PASS)?;

    verify_record(paths, &root, &record)?;
    crate::git::resolved()?.require_unchanged()?;
    let manifest = root.join(MANIFEST_NAME);
    write_manifest(&root, &manifest, &record.render())?;
    println!("[xtask] release evidence ready at {}", manifest.display());
    Ok(())
}

fn reproduce_release(paths: &Paths, target: &Path, label: &str) -> Result<ReproducedRelease> {
    let build = |source: &Path| {
        let copied = Paths::new(source.to_path_buf(), Some(target));
        let image = image::build(&copied, Profile::Release, image::Kernel::Shipped)
            .with_context(|| format!("building clean release path {label}"))?;
        Ok(ReproducedRelease {
            image,
            kernel: copied.kernel_elf(Profile::Release),
            host_roots: crate::cargo::active_kernel_remap_roots(&copied)?,
        })
    };
    crate::mutation_workspace::with_clean_source_copy(paths, build)
}

fn reproduce_release_alternate(
    paths: &Paths,
    evidence: &Path,
    target: &Path,
) -> Result<ReproducedRelease> {
    let original = crate::cargo::active_kernel_remap_roots(paths)?;
    let toolchain = evidence.join("toolchain-b");
    let cargo_home = evidence.join("cargo-home-b");
    let result = (|| {
        let mut entries = 0usize;
        for child in ["bin", "lib"] {
            mirror_directory(
                &original[1].join(child),
                &toolchain.join(child),
                0,
                &mut entries,
            )?;
        }
        mirror_directory(
            &original[2].join("registry"),
            &cargo_home.join("registry"),
            0,
            &mut entries,
        )?;
        for name in ["config", "config.toml"] {
            let source = original[2].join(name);
            match std::fs::symlink_metadata(&source) {
                Ok(metadata) => {
                    entries = entries
                        .checked_add(1)
                        .context("alternate build mirror entry count overflowed")?;
                    ensure!(
                        entries <= MIRROR_ENTRY_LIMIT,
                        "alternate build mirror exceeds its entry limit"
                    );
                    mirror_file(&source, &cargo_home.join(name), &metadata)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("inspecting {}", source.display()));
                }
            }
        }

        crate::mutation_workspace::with_clean_source_copy(paths, |source| {
            run_alternate_release_build(source, target, &toolchain, &cargo_home)?;
            let copied = Paths::new(source.to_path_buf(), Some(target));
            let image = copied.image();
            let kernel = copied.kernel_elf(Profile::Release);
            crate::elf::verify_file(&kernel)?;
            ensure!(image.is_file(), "alternate release build produced no image");
            Ok(ReproducedRelease {
                image,
                kernel,
                host_roots: [
                    crate::mutation_workspace::canonical_existing_real_directory(
                        source,
                        "alternate release source root",
                    )?,
                    crate::mutation_workspace::canonical_existing_real_directory(
                        &toolchain,
                        "alternate release toolchain root",
                    )?,
                    crate::mutation_workspace::canonical_existing_real_directory(
                        &cargo_home,
                        "alternate release Cargo home",
                    )?,
                ],
            })
        })
    })();
    let toolchain_cleanup = remove_child(evidence, "toolchain-b");
    let cargo_cleanup = remove_child(evidence, "cargo-home-b");
    match (result, toolchain_cleanup, cargo_cleanup) {
        (Ok(release), Ok(()), Ok(())) => Ok(release),
        (Err(error), Ok(()), Ok(())) => Err(error),
        (result, toolchain_cleanup, cargo_cleanup) => {
            let primary = match result {
                Ok(_) => "alternate release build passed".to_string(),
                Err(error) => format!("{error:#}"),
            };
            Err(anyhow::anyhow!(
                "{primary}; alternate-root cleanup failed: toolchain={}; cargo-home={}",
                cleanup_outcome(toolchain_cleanup),
                cleanup_outcome(cargo_cleanup)
            ))
        }
    }
}

fn cleanup_outcome(result: Result<()>) -> String {
    result.map_or_else(|error| format!("{error:#}"), |()| "ok".to_string())
}

fn run_alternate_release_build(
    source: &Path,
    target: &Path,
    toolchain: &Path,
    cargo_home: &Path,
) -> Result<()> {
    let executable = |name: &str| {
        toolchain.join("bin").join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        })
    };
    let inherited = std::env::var_os("PATH").context("PATH is unavailable")?;
    let path = std::env::join_paths(
        std::iter::once(toolchain.join("bin")).chain(std::env::split_paths(&inherited)),
    )
    .context("constructing alternate release PATH")?;
    let mut command = Command::new(executable("cargo"));
    for (name, _) in std::env::vars_os() {
        let normalized = name.to_string_lossy().to_ascii_uppercase();
        if normalized == "CARGO"
            || normalized.starts_with("CARGO_")
            || normalized.starts_with("RUST")
        {
            command.env_remove(name);
        }
    }
    command
        .current_dir(source)
        .env("CARGO_HOME", crate::cargo::child_process_path(cargo_home))
        .env("CARGO_TARGET_DIR", crate::cargo::child_process_path(target))
        .env("CARGO_NET_OFFLINE", "true")
        .env("RUSTC", executable("rustc"))
        .env("RUSTDOC", executable("rustdoc"))
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env("PATH", path)
        .args([
            "run",
            "--offline",
            "--locked",
            "--profile",
            "xtask",
            "--package",
            "xtask",
            "--",
            "image",
        ]);
    let output = bounded_process::output_with_limit(
        command,
        ALTERNATE_BUILD_TIMEOUT,
        "alternate-root release build",
        ALTERNATE_BUILD_OUTPUT_BYTES,
    )?;
    std::io::stdout()
        .write_all(&output.stdout)
        .context("forwarding alternate release output")?;
    std::io::stderr()
        .write_all(&output.stderr)
        .context("forwarding alternate release diagnostics")?;
    ensure!(
        output.status.success(),
        "alternate-root release build failed ({})",
        output.status
    );
    Ok(())
}

fn mirror_directory(
    source: &Path,
    destination: &Path,
    depth: usize,
    entries: &mut usize,
) -> Result<()> {
    ensure!(
        depth <= MIRROR_DEPTH_LIMIT,
        "alternate build mirror exceeds its directory-depth limit"
    );
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("inspecting mirror source {}", source.display()))?;
    ensure!(
        metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && !is_reparse(&metadata),
        "alternate build mirror source is not a real directory: {}",
        source.display()
    );
    std::fs::create_dir(destination)
        .with_context(|| format!("creating mirror directory {}", destination.display()))?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("reading an entry in {}", source.display()))?;
        *entries = entries
            .checked_add(1)
            .context("alternate build mirror entry count overflowed")?;
        ensure!(
            *entries <= MIRROR_ENTRY_LIMIT,
            "alternate build mirror exceeds its entry limit"
        );
        let metadata = std::fs::symlink_metadata(entry.path())
            .with_context(|| format!("inspecting {}", entry.path().display()))?;
        let target = destination.join(entry.file_name());
        if metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && !is_reparse(&metadata)
        {
            mirror_directory(&entry.path(), &target, depth + 1, entries)?;
        } else {
            mirror_file(&entry.path(), &target, &metadata)?;
        }
    }
    Ok(())
}

fn mirror_file(source: &Path, destination: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    ensure!(
        metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && !is_reparse(metadata),
        "alternate build mirror source is not a real file: {}",
        source.display()
    );
    std::fs::hard_link(source, destination).with_context(|| {
        format!(
            "hard-linking alternate build input {} to {}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
const fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn require_no_host_paths(path: &Path, maximum: u64, label: &str, roots: &[PathBuf]) -> Result<()> {
    let bytes = qualification::read_bounded_file(path, maximum, label)?;
    for root in roots {
        let root = crate::cargo::child_process_path(root);
        let text = root
            .to_str()
            .with_context(|| format!("{label} host root is not UTF-8: {}", root.display()))?;
        for spelling in [text.to_string(), text.replace('\\', "/")] {
            ensure!(
                !spelling.is_empty()
                    && !bytes
                        .windows(spelling.len())
                        .any(|window| window == spelling.as_bytes()),
                "{label} retains host path {spelling:?}"
            );
        }
    }
    Ok(())
}

fn relation_requires_current_mutation(policy: &str) -> bool {
    policy == "same-commit"
}

/// Verify the evidence bundle against the current clean checkout and installed
/// tools without contacting a network or rebuilding anything.
pub(crate) fn verify(paths: &Paths) -> Result<()> {
    let root = evidence_root(paths);
    let manifest = root.join(MANIFEST_NAME);
    require_contained_regular_file(&root, &manifest, "evidence manifest")?;
    let text =
        qualification::read_bounded_text(&manifest, MAX_MANIFEST_BYTES, "evidence manifest")?;
    let record = parse_manifest(&text)?;

    verify_record(paths, &root, &record)?;
    println!("[xtask] evidence manifest and every recorded byte verify offline");
    Ok(())
}

fn verify_record(paths: &Paths, root: &Path, record: &Record) -> Result<()> {
    vendor::verify(paths)?;
    let qualified_source = qualification::SourceIdentity {
        commit: record.get("qualification.source.commit")?.to_string(),
        tree: record.get("qualification.source.tree")?.to_string(),
    };
    let observed = input_record_with_relation(paths, &qualified_source)?;
    let mut expected_keys: BTreeSet<String> = observed.fields.keys().cloned().collect();
    for spec in ARTIFACTS {
        expected_keys.insert(format!("{}.path", spec.prefix));
        expected_keys.insert(format!("{}.bytes", spec.prefix));
        expected_keys.insert(format!("{}.sha256", spec.prefix));
    }
    for spec in qualification::BUNDLE_FILES {
        expected_keys.insert(format!("{}.path", spec.prefix));
        expected_keys.insert(format!("{}.bytes", spec.prefix));
        expected_keys.insert(format!("{}.sha256", spec.prefix));
    }
    for step in gate::REGISTRY.steps() {
        expected_keys.insert(step.evidence_key().to_string());
    }
    expected_keys.insert("result.artifacts-byte-identical".to_string());
    expected_keys.insert("result.shipped-artifact-boot".to_string());
    require_exact_keys(record, &expected_keys)?;

    require_inputs_match(record, &observed)?;
    verify_artifacts(record, root)?;
    verify_qualification_files(record, root)?;
    verify_qualification_artifact_identity(paths, root)?;
    qualification::verify_bundle(
        paths,
        root,
        &qualified_source,
        &mutate::registry_fingerprint(),
        mutate::MUTATIONS.len(),
    )?;
    require_pass_results(record)?;
    Ok(())
}

fn input_record_with_relation(
    paths: &Paths,
    qualified: &qualification::SourceIdentity,
) -> Result<Record> {
    let mut record = input_record(paths)?;
    QualificationRelation::discover(paths, qualified)?.add_to(&mut record)?;
    Ok(record)
}

fn input_record(paths: &Paths) -> Result<Record> {
    let mut record = Record::default();
    let (commit, tree) = clean_source(paths.root())?;
    record.insert("source.commit", commit)?;
    record.insert("source.tree", tree)?;
    for (key, value) in crate::git::resolved()?.fields() {
        record.insert(key, value)?;
    }
    add_digest_bounded(
        &mut record,
        "source.cargo-lock",
        &paths.root().join("Cargo.lock"),
        MAX_SOURCE_LOCK_BYTES,
        "source Cargo.lock",
    )?;
    add_digest_bounded(
        &mut record,
        "source.rust-toolchain",
        &paths.root().join("rust-toolchain.toml"),
        MAX_TOOLCHAIN_FILE_BYTES,
        "source toolchain file",
    )?;

    record.insert(
        "tool.rustc.version",
        trusted_version_output(paths.root(), Path::new("rustc"), &["-vV"])
            .context("reading the Rust compiler version")?,
    )?;
    record.insert(
        "tool.cargo.version",
        trusted_version_output(paths.root(), Path::new(env!("CARGO")), &["-Vv"])
            .context("reading the Cargo version")?,
    )?;
    let qemu = qemu::installation()?;
    record.insert(
        "tool.qemu.version",
        qemu_version_output(paths.root(), &qemu.emulator).context("reading the QEMU version")?,
    )?;
    record.insert("tool.qemu.machine", qemu::MACHINE_TYPE)?;
    add_digest_bounded(
        &mut record,
        "tool.qemu",
        &qemu.emulator,
        MAX_TOOL_BINARY_BYTES,
        "QEMU executable",
    )?;
    add_digest_bounded(
        &mut record,
        "firmware.code",
        &qemu.code_firmware,
        MAX_FIRMWARE_BYTES,
        "QEMU code firmware",
    )?;
    add_digest_bounded(
        &mut record,
        "firmware.vars-template",
        &qemu.vars_firmware,
        MAX_FIRMWARE_BYTES,
        "QEMU variable firmware",
    )?;
    add_known_digest(
        &mut record,
        "input.nvme",
        digest_file_exact(
            &qemu::nvme_path(paths.root()),
            namespace_bytes(),
            "NVMe namespace input",
        )?,
    )?;
    add_optional_tool(&mut record, "tool.java", qualification::java(paths.root())?)?;
    add_optional_tool(
        &mut record,
        "tool.vmware",
        qualification::vmrun(paths.root())?,
    )?;
    add_vendor_inputs(&mut record, paths)?;
    Ok(record)
}

fn add_optional_tool(
    record: &mut Record,
    prefix: &str,
    tool: Option<qualification::Tool>,
) -> Result<()> {
    let identity = match tool {
        Some(tool) => ToolIdentity::Available {
            version: tool.version,
            digest: digest_file_bounded(
                &tool.path,
                MAX_TOOL_BINARY_BYTES,
                "qualification tool executable",
            )?,
        },
        None => ToolIdentity::Unavailable,
    };
    add_tool_identity(record, prefix, &identity)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ToolIdentity {
    Available { version: String, digest: Digest },
    Unavailable,
}

fn add_tool_identity(record: &mut Record, prefix: &str, identity: &ToolIdentity) -> Result<()> {
    let (state, version, bytes, sha256) = match identity {
        ToolIdentity::Available { version, digest } => (
            AVAILABLE,
            version.replace('\n', "\\n"),
            digest.bytes.to_string(),
            digest.sha256.clone(),
        ),
        ToolIdentity::Unavailable => (
            UNAVAILABLE,
            UNAVAILABLE.to_string(),
            UNAVAILABLE.to_string(),
            UNAVAILABLE.to_string(),
        ),
    };
    record.insert(format!("{prefix}.state"), state)?;
    record.insert(format!("{prefix}.version"), version)?;
    record.insert(format!("{prefix}.bytes"), bytes)?;
    record.insert(format!("{prefix}.sha256"), sha256)?;
    Ok(())
}

fn require_inputs_match(record: &Record, observed: &Record) -> Result<()> {
    for prefix in QUALIFICATION_TOOLS {
        let key = format!("{prefix}.state");
        let expected = record.get(&key)?;
        let actual = observed.get(&key)?;
        ensure!(
            expected == actual,
            "qualification dependency `{prefix}` differs: manifest `{expected}`, current `{actual}`"
        );
    }
    for (key, actual) in &observed.fields {
        let expected = record.get(key)?;
        ensure!(
            expected == actual,
            "evidence input `{key}` differs: manifest `{expected}`, current `{actual}`"
        );
    }
    Ok(())
}

fn add_vendor_inputs(record: &mut Record, paths: &Paths) -> Result<()> {
    for (label, listing) in [
        ("limine", paths.limine_checksums()),
        ("tla", paths.tla_checksums()),
        ("fat32-corpus", paths.fat32_corpus_checksums()),
    ] {
        add_digest_bounded(
            record,
            &format!("vendor.{label}.listing"),
            &listing,
            vendor::MAX_LISTING_BYTES,
            "vendor checksum listing",
        )?;
        let text = vendor::read_listing(&listing)?;
        let entries =
            vendor::parse(&text).with_context(|| format!("parsing {}", listing.display()))?;
        let directory = listing
            .parent()
            .context("vendor listing has no directory")?;
        for (index, entry) in entries.iter().enumerate() {
            let prefix = format!("vendor.{label}.payload.{:02}", index + 1);
            record.insert(format!("{prefix}.name"), entry.name.clone())?;
            add_digest_bounded(
                record,
                &prefix,
                &directory.join(&entry.name),
                vendor::MAX_PAYLOAD_BYTES,
                "vendored evidence payload",
            )?;
        }
    }
    Ok(())
}

fn add_artifacts(record: &mut Record, root: &Path) -> Result<()> {
    add_artifacts_with_image_bytes(record, root, boot_image_bytes())
}

fn add_artifacts_with_image_bytes(
    record: &mut Record,
    root: &Path,
    expected_image_bytes: u64,
) -> Result<()> {
    for spec in ARTIFACTS {
        record.insert(format!("{}.path", spec.prefix), spec.relative)?;
        let digest = if spec.prefix.ends_with(".image") {
            digest_bundle_file_exact(root, spec.relative, spec.prefix, expected_image_bytes)?
        } else {
            digest_bundle_file(root, spec.relative, spec.prefix)?
        };
        add_known_digest(record, spec.prefix, digest)?;
    }
    Ok(())
}

fn add_qualification_files(record: &mut Record, root: &Path) -> Result<()> {
    for spec in qualification::BUNDLE_FILES {
        record.insert(format!("{}.path", spec.prefix), spec.relative)?;
        let digest = verified_qualification_digest(root, spec)?;
        add_known_digest(record, spec.prefix, digest)?;
    }
    Ok(())
}

fn verify_artifacts(record: &Record, root: &Path) -> Result<()> {
    verify_artifacts_with_image_bytes(record, root, boot_image_bytes())
}

fn verify_artifacts_with_image_bytes(
    record: &Record,
    root: &Path,
    expected_image_bytes: u64,
) -> Result<()> {
    let mut digests = Vec::new();
    for spec in ARTIFACTS {
        ensure!(
            record.get(&format!("{}.path", spec.prefix))? == spec.relative,
            "{} names an unexpected bundle path",
            spec.prefix
        );
        let digest = if spec.prefix.ends_with(".image") {
            digest_bundle_file_exact(root, spec.relative, spec.prefix, expected_image_bytes)?
        } else {
            digest_bundle_file(root, spec.relative, spec.prefix)?
        };
        ensure!(
            record.get(&format!("{}.bytes", spec.prefix))? == digest.bytes.to_string(),
            "{} has a different length than its manifest",
            spec.prefix
        );
        ensure!(
            record.get(&format!("{}.sha256", spec.prefix))? == digest.sha256,
            "{} has a different digest than its manifest",
            spec.prefix
        );
        digests.push(digest);
    }
    ensure!(
        byte_identical(&digests[0], &digests[2]),
        "the bundled kernel paths are not byte-identical"
    );
    ensure!(
        byte_identical(&digests[1], &digests[3]),
        "the bundled image paths are not byte-identical"
    );
    Ok(())
}

fn verify_qualification_files(record: &Record, root: &Path) -> Result<()> {
    for spec in qualification::BUNDLE_FILES {
        ensure!(
            record.get(&format!("{}.path", spec.prefix))? == spec.relative,
            "{} names an unexpected bundle path",
            spec.prefix
        );
        let digest = verified_qualification_digest(root, spec)?;
        ensure!(
            record.get(&format!("{}.bytes", spec.prefix))? == digest.bytes.to_string(),
            "{} has a different length than its manifest",
            spec.prefix
        );
        ensure!(
            record.get(&format!("{}.sha256", spec.prefix))? == digest.sha256,
            "{} has a different digest than its manifest",
            spec.prefix
        );
    }
    Ok(())
}

fn verify_qualification_artifact_identity(paths: &Paths, root: &Path) -> Result<()> {
    verify_qualification_artifact_identity_with_sizes(
        paths,
        root,
        boot_image_bytes(),
        namespace_bytes(),
    )
}

fn verify_qualification_artifact_identity_with_sizes(
    paths: &Paths,
    root: &Path,
    expected_image_bytes: u64,
    expected_namespace_bytes: u64,
) -> Result<()> {
    let selftest_image = digest_bundle_file_exact(
        root,
        "qualification/vmware-self-test/bedrock-flat.img",
        "VMware self-test image",
        expected_image_bytes,
    )?;
    let shipped_image = digest_bundle_file_exact(
        root,
        "qualification/vmware-shipped/bedrock-flat.img",
        "VMware shipped image",
        expected_image_bytes,
    )?;
    ensure!(
        shipped_image
            == digest_bundle_file_exact(
                root,
                ARTIFACTS[1].relative,
                ARTIFACTS[1].prefix,
                expected_image_bytes,
            )?,
        "the VMware shipped image differs from the reproduced release"
    );
    ensure!(
        selftest_image != shipped_image,
        "the VMware self-test and shipped selectors produced the same image"
    );

    let namespace = digest_file_exact(
        &qemu::nvme_path(paths.root()),
        expected_namespace_bytes,
        "current storage namespace",
    )?;
    for kind in ["vmware-self-test", "vmware-shipped"] {
        let bundled = digest_bundle_file_exact(
            root,
            &format!("qualification/{kind}/wizard-nvm-flat.img"),
            "VMware namespace",
            expected_namespace_bytes,
        )?;
        ensure!(
            bundled == namespace,
            "the {kind} namespace differs from the recorded boot input"
        );
    }
    Ok(())
}

fn verified_qualification_digest(root: &Path, spec: &qualification::BundleFile) -> Result<Digest> {
    verified_qualification_digest_with(root, spec, || Ok(()))
}

fn verified_qualification_digest_with<F>(
    root: &Path,
    spec: &qualification::BundleFile,
    after_path_check: F,
) -> Result<Digest>
where
    F: FnOnce() -> Result<()>,
{
    let path = checked_bundle_file(root, spec.relative, spec.prefix)?;
    after_path_check()?;
    let (maximum, exact) = qualification::bundle_file_size_policy(spec)?;
    if exact {
        digest_file_exact(&path, maximum, spec.prefix)
    } else {
        digest_file_bounded(&path, maximum, spec.prefix)
    }
}

fn require_pass_results(record: &Record) -> Result<()> {
    for key in gate::REGISTRY
        .steps()
        .iter()
        .map(|step| step.evidence_key())
        .chain([
            "result.artifacts-byte-identical",
            "result.shipped-artifact-boot",
        ])
    {
        ensure!(
            record.get(key)? == PASS,
            "evidence result `{key}` is not pass"
        );
    }
    Ok(())
}

fn require_exact_keys(record: &Record, expected: &BTreeSet<String>) -> Result<()> {
    let actual: BTreeSet<String> = record.fields.keys().cloned().collect();
    ensure!(
        actual == *expected,
        "evidence schema differs: missing {:?}; unknown {:?}",
        expected.difference(&actual).collect::<Vec<_>>(),
        actual.difference(expected).collect::<Vec<_>>()
    );
    Ok(())
}

fn parse_manifest(text: &str) -> Result<Record> {
    ensure!(
        !text.contains('\r'),
        "evidence manifest must use LF line endings"
    );
    let mut lines = text.lines();
    ensure!(
        core::iter::Iterator::next(&mut lines) == Some(FORMAT),
        "evidence manifest has no supported format header"
    );
    let mut record = Record::default();
    for line in lines {
        ensure!(!line.is_empty(), "evidence manifest contains a blank line");
        let (key, value) = line
            .split_once('=')
            .with_context(|| format!("evidence line has no value: {line}"))?;
        record.insert(key, value)?;
    }
    ensure!(!record.fields.is_empty(), "evidence manifest is empty");
    ensure!(
        record.render() == text,
        "evidence manifest is not in its canonical sorted encoding"
    );
    Ok(record)
}

fn validate_field(key: &str, value: &str) -> Result<()> {
    ensure!(
        !key.is_empty()
            && key.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b".-".contains(&byte)),
        "invalid evidence key `{key}`"
    );
    ensure!(
        !value.contains(['\r', '\n', '\0']),
        "evidence value for `{key}` contains a forbidden byte"
    );
    Ok(())
}

fn clean_source(root: &Path) -> Result<(String, String)> {
    let before_status = git_output(
        root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        MAX_GIT_STATUS_BYTES,
        "evidence source status",
    )?;
    let before = git_source_identity(root, "evidence source")?;
    let after_status = git_output(
        root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        MAX_GIT_STATUS_BYTES,
        "settled evidence source status",
    )?;
    let after = git_source_identity(root, "settled evidence source")?;
    require_stable_clean_source(&before_status, &before, &after_status, &after)?;
    Ok(after)
}

fn git_source_identity(root: &Path, label: &str) -> Result<(String, String)> {
    let commit = git_output(
        root,
        &["rev-parse", "HEAD"],
        MAX_GIT_IDENTITY_BYTES,
        &format!("{label} commit"),
    )?;
    let tree = git_output(
        root,
        &["rev-parse", "HEAD^{tree}"],
        MAX_GIT_IDENTITY_BYTES,
        &format!("{label} tree"),
    )?;
    ensure!(is_git_id(&commit), "Git returned an invalid commit ID");
    ensure!(is_git_id(&tree), "Git returned an invalid tree ID");
    Ok((commit, tree))
}

fn require_stable_clean_source(
    before_status: &str,
    before: &(String, String),
    after_status: &str,
    after: &(String, String),
) -> Result<()> {
    ensure!(
        status_is_clean(before_status) && status_is_clean(after_status),
        "release evidence requires a stable clean checkout:\n{before_status}{after_status}"
    );
    ensure!(
        before == after,
        "release source identity changed while it was captured"
    );
    Ok(())
}

fn status_is_clean(status: &str) -> bool {
    status.trim().is_empty()
}

fn is_git_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_output(root: &Path, args: &[&str], maximum: usize, label: &str) -> Result<String> {
    let mut command = crate::git::resolved()?.command(root);
    command.args(args);
    let output = bounded_process::output_with_limit(command, IDENTITY_DEADLINE, label, maximum)
        .with_context(|| format!("running git {}", args.join(" ")))?;
    normalize_command_output(Path::new("git"), args, output)
}

fn git_output_bytes(root: &Path, args: &[&str], maximum: usize, label: &str) -> Result<Vec<u8>> {
    let mut command = crate::git::resolved()?.command(root);
    command.args(args);
    let output = bounded_process::output_with_limit(command, IDENTITY_DEADLINE, label, maximum)
        .with_context(|| format!("running git {}", args.join(" ")))?;
    ensure!(
        output.status.success() && output.stderr.is_empty(),
        "git {} failed or wrote diagnostics",
        args.join(" ")
    );
    Ok(output.stdout)
}

fn parse_parent_docs_diff(raw: &[u8]) -> Result<Vec<String>> {
    const ALLOWED: &[&str] = &[
        "README.md",
        "docs/004-decision-log.md",
        "docs/014-storage.md",
        "docs/016-pre-wizzy-foundation.md",
        "docs/017-pre-wizzy-hostile-surface-audit.md",
    ];
    ensure!(
        !raw.is_empty() && raw.last() == Some(&0),
        "parent documentation diff is empty or not NUL terminated"
    );
    let fields: Vec<_> = raw[..raw.len() - 1].split(|byte| *byte == 0).collect();
    ensure!(
        fields.len() % 2 == 0,
        "parent documentation diff has an incomplete row"
    );
    let mut files = Vec::new();
    for row in fields.chunks_exact(2) {
        let header = std::str::from_utf8(row[0]).context("parent diff header is not UTF-8")?;
        let path = std::str::from_utf8(row[1]).context("parent diff path is not UTF-8")?;
        let fields: Vec<_> = header.split_whitespace().collect();
        ensure!(
            fields.len() == 5
                && fields[0] == ":100644"
                && fields[1] == "100644"
                && fields[2].len() == 40
                && fields[3].len() == 40
                && fields[2].bytes().all(|byte| byte.is_ascii_hexdigit())
                && fields[3].bytes().all(|byte| byte.is_ascii_hexdigit())
                && fields[4] == "M",
            "parent-docs-only-v1 permits only ordinary 100644 file modifications"
        );
        ensure!(
            ALLOWED.contains(&path),
            "parent-docs-only-v1 refuses `{path}`"
        );
        ensure!(
            files
                .last()
                .is_none_or(|previous: &String| previous.as_str() < path),
            "parent documentation paths are not unique and sorted"
        );
        files.push(path.to_string());
    }
    Ok(files)
}

fn trusted_command_output(root: &Path, program: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new(program);
    command.current_dir(root).args(args);
    let label = format!("{} version identity probe", program.display());
    let output = bounded_process::output_with_limit(
        command,
        IDENTITY_DEADLINE,
        &label,
        MAX_TOOL_VERSION_BYTES,
    )?;
    normalize_command_output(program, args, output)
}

fn normalize_command_output(program: &Path, args: &[&str], output: Output) -> Result<String> {
    ensure!(
        output.status.success(),
        "{} {} failed ({})\n{}",
        program.display(),
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let stdout = String::from_utf8(output.stdout)
        .with_context(|| format!("{} output is not UTF-8", program.display()))?;
    let stderr = String::from_utf8(output.stderr)
        .with_context(|| format!("{} diagnostic output is not UTF-8", program.display()))?;
    let mut combined = stdout.trim().replace("\r\n", "\n").replace('\r', "\n");
    if !stderr.trim().is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("stderr: ");
        combined.push_str(stderr.trim());
    }
    Ok(combined)
}

fn trusted_version_output(root: &Path, program: &Path, args: &[&str]) -> Result<String> {
    let output = trusted_command_output(root, program, args)?;
    require_version_output(program, output)
}

fn qemu_version_output(root: &Path, program: &Path) -> Result<String> {
    qemu_version_output_with(root, program, |command, deadline, label| {
        bounded_process::output(command, deadline, label)
    })
}

fn qemu_version_output_with<F>(root: &Path, program: &Path, run: F) -> Result<String>
where
    F: FnOnce(Command, Duration, &str) -> Result<Output>,
{
    let args = ["--version"];
    let mut command = Command::new(program);
    command.current_dir(root).args(args);
    let output = run(command, QEMU_VERSION_DEADLINE, QEMU_VERSION_LABEL)?;
    let output = normalize_command_output(program, &args, output)?;
    require_version_output(program, output)
}

fn require_version_output(program: &Path, output: String) -> Result<String> {
    ensure!(
        !output.is_empty(),
        "{} printed no version",
        program.display()
    );
    Ok(output.replace('\n', "\\n"))
}

fn add_digest_bounded(
    record: &mut Record,
    prefix: &str,
    path: &Path,
    maximum: u64,
    label: &str,
) -> Result<()> {
    let digest = digest_file_bounded(path, maximum, label)?;
    add_known_digest(record, prefix, digest)
}

fn add_known_digest(record: &mut Record, prefix: &str, digest: Digest) -> Result<()> {
    record.insert(format!("{prefix}.bytes"), digest.bytes.to_string())?;
    record.insert(format!("{prefix}.sha256"), digest.sha256)?;
    Ok(())
}

fn digest_file_bounded(path: &Path, maximum: u64, label: &str) -> Result<Digest> {
    digest_file_bounded_with(path, maximum, label, || Ok(()), || Ok(()))
}

fn digest_file_bounded_with<F, G>(
    path: &Path,
    maximum: u64,
    label: &str,
    after_measurement: F,
    after_hash: G,
) -> Result<Digest>
where
    F: FnOnce() -> Result<()>,
    G: FnOnce() -> Result<()>,
{
    let parent = path
        .parent()
        .with_context(|| format!("{label} has no parent directory"))?;
    crate::mutation_workspace::canonical_existing_real_directory(parent, label)?;
    crate::mutation_workspace::require_direct_file(parent, path, label)?;
    let file = crate::mutation_workspace::open_real_file(path, label)?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {label} {}", path.display()))?
        .len();
    ensure!(
        initial <= maximum,
        "{label} is {initial} bytes, exceeding its {maximum}-byte hashing limit"
    );
    after_measurement()?;
    let (bytes, digest) = sha256::digest_reader(file.take(maximum + 1))
        .with_context(|| format!("hashing {label} {}", path.display()))?;
    ensure!(
        bytes <= maximum,
        "{label} grew beyond its {maximum}-byte hashing limit while it was hashed"
    );
    ensure!(
        bytes == initial,
        "{label} changed length while it was hashed"
    );
    after_hash()?;
    crate::mutation_workspace::require_direct_file(parent, path, label)?;
    ensure!(
        crate::mutation_workspace::open_real_file(path, label)?
            .metadata()
            .with_context(|| format!("remeasuring {label} {}", path.display()))?
            .len()
            == initial,
        "{label} changed length after it was hashed"
    );
    Ok(Digest {
        bytes,
        sha256: sha256::hex(&digest),
    })
}

fn digest_file_exact(path: &Path, expected: u64, label: &str) -> Result<Digest> {
    digest_file_exact_with(path, expected, label, |file| {
        let (bytes, digest) = sha256::digest_reader(file)
            .with_context(|| format!("hashing {label} {}", path.display()))?;
        Ok(Digest {
            bytes,
            sha256: sha256::hex(&digest),
        })
    })
}

fn digest_file_exact_with<F>(path: &Path, expected: u64, label: &str, hash: F) -> Result<Digest>
where
    F: FnOnce(&mut std::io::Take<File>) -> Result<Digest>,
{
    let parent = path
        .parent()
        .with_context(|| format!("{label} has no parent directory"))?;
    crate::mutation_workspace::canonical_existing_real_directory(parent, label)?;
    crate::mutation_workspace::require_direct_file(parent, path, label)?;
    let file = crate::mutation_workspace::open_real_file(path, label)?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {label} {}", path.display()))?
        .len();
    ensure!(
        initial == expected,
        "{label} is {initial} bytes, expected exactly {expected}"
    );
    let mut limited = file.take(expected + 1);
    let digest = hash(&mut limited)?;
    crate::mutation_workspace::require_direct_file(parent, path, label)?;
    ensure!(
        digest.bytes == expected,
        "{label} hashed {} bytes, expected exactly {expected}",
        digest.bytes
    );
    ensure!(
        limited
            .get_ref()
            .metadata()
            .with_context(|| format!("remeasuring {label} {}", path.display()))?
            .len()
            == expected,
        "{label} changed outside its exact {expected}-byte extent while being hashed"
    );
    Ok(digest)
}

fn boot_image_bytes() -> u64 {
    crate::paths::DISK_MIB * 1024 * 1024
}

fn namespace_bytes() -> u64 {
    crate::qemu::NVME_VOLUME_SECTOR * crate::qemu::NVME_BLOCK_BYTES as u64
        + crate::image::corpus::VOLUME_BYTES as u64
}

fn byte_identical(first: &Digest, second: &Digest) -> bool {
    first.bytes == second.bytes && first.sha256 == second.sha256
}

fn copy_exact(source: &Path, destination: &Path, expected: &Digest) -> Result<()> {
    let mut input = crate::mutation_workspace::open_real_file(source, "evidence source artifact")?;
    let mut output_options = OpenOptions::new();
    OpenOptions::create_new(&mut output_options, true);
    OpenOptions::write(&mut output_options, true);
    let mut output = OpenOptions::open(&output_options, destination)
        .with_context(|| format!("creating {}", destination.display()))?;
    let copied = std::io::copy(&mut (&mut input).take(expected.bytes + 1), &mut output)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    ensure!(
        copied == expected.bytes
            && input
                .metadata()
                .with_context(|| format!("remeasuring {}", source.display()))?
                .len()
                == expected.bytes,
        "{} changed length while entering the evidence bundle",
        source.display()
    );
    output
        .sync_all()
        .with_context(|| format!("syncing {}", destination.display()))?;
    drop(output);
    ensure!(
        digest_file_exact(destination, expected.bytes, "copied evidence artifact")? == *expected,
        "{} changed while entering the evidence bundle",
        source.display()
    );
    Ok(())
}

fn evidence_root(paths: &Paths) -> PathBuf {
    paths.target_dir().join("evidence")
}

fn digest_bundle_file(root: &Path, relative: &str, label: &str) -> Result<Digest> {
    let path = checked_bundle_file(root, relative, label)?;
    digest_file_bounded(&path, MAX_KERNEL_BYTES, label)
}

fn digest_bundle_file_exact(
    root: &Path,
    relative: &str,
    label: &str,
    expected: u64,
) -> Result<Digest> {
    let path = checked_bundle_file(root, relative, label)?;
    digest_file_exact(&path, expected, label)
}

fn checked_bundle_file(root: &Path, relative: &str, label: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "{label} has an unsafe bundle path"
    );
    let path = root.join(relative);
    require_contained_regular_file(root, &path, label)?;
    Ok(path)
}

fn require_real_directory(path: &Path, label: &str) -> Result<()> {
    crate::mutation_workspace::canonical_existing_real_directory(path, label)?;
    Ok(())
}

fn require_evidence_root(root: &Path) -> Result<()> {
    require_real_directory(root, "evidence root")?;
    let parent = root.parent().context("evidence root has no parent")?;
    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalizing {}", parent.display()))?;
    let canonical_root = std::fs::canonicalize(root)
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    ensure!(
        canonical_root.parent() == Some(canonical_parent.as_path()),
        "evidence root escaped its canonical parent"
    );
    Ok(())
}

fn require_contained_real_directory(root: &Path, directory: &Path, label: &str) -> Result<()> {
    require_evidence_root(root)?;
    let relative = directory
        .strip_prefix(root)
        .with_context(|| format!("{label} escaped {}", root.display()))?;
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "{label} has an unsafe relative path"
    );
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        require_real_directory(&current, label)?;
    }
    let canonical_root = std::fs::canonicalize(root)
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let canonical_directory = std::fs::canonicalize(directory)
        .with_context(|| format!("canonicalizing {}", directory.display()))?;
    ensure!(
        canonical_directory.starts_with(&canonical_root) && canonical_directory != canonical_root,
        "{label} escaped the canonical evidence root"
    );
    Ok(())
}

fn require_contained_regular_file(root: &Path, path: &Path, label: &str) -> Result<()> {
    require_evidence_root(root)?;
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{label} escaped {}", root.display()))?;
    let components: Vec<_> = relative.components().collect();
    ensure!(
        !components.is_empty()
            && components
                .iter()
                .all(|component| matches!(component, Component::Normal(_))),
        "{label} has an unsafe relative path"
    );
    let mut current = root.to_path_buf();
    for component in &components[..components.len() - 1] {
        current.push(component.as_os_str());
        require_real_directory(&current, label)?;
    }
    let parent = path.parent().context("bundle file has no parent")?;
    crate::mutation_workspace::require_direct_file(parent, path, label)?;
    let canonical_root = std::fs::canonicalize(root)
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let canonical_path = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalizing {}", path.display()))?;
    ensure!(
        canonical_path.starts_with(canonical_root),
        "{label} escaped the canonical evidence root"
    );
    Ok(())
}

fn invalidate_manifest(paths: &Paths) -> Result<PathBuf> {
    let root = crate::mutation_workspace::prepare_real_directory_tree(
        &evidence_root(paths),
        "evidence root",
    )?;
    require_evidence_root(&root)?;
    for name in [MANIFEST_NAME, MANIFEST_TEMP] {
        let path = root.join(name);
        ensure!(
            path.parent() == Some(root.as_path()),
            "evidence manifest escaped {}",
            root.display()
        );
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                anyhow::bail!("evidence manifest entry is unexpectedly a directory")
            }
            Ok(_) => {
                std::fs::remove_file(&path).with_context(|| {
                    format!("invalidating stale evidence manifest {}", path.display())
                })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", path.display()));
            }
        }
    }
    Ok(root)
}

fn reset_child(root: &Path, name: &str) -> Result<()> {
    remove_child(root, name)?;
    let child = checked_child(root, name)?;
    std::fs::create_dir_all(&child).with_context(|| format!("creating {}", child.display()))?;
    require_contained_real_directory(root, &child, "evidence child")
}

fn remove_child(root: &Path, name: &str) -> Result<()> {
    require_evidence_root(root)?;
    let child = checked_child(root, name)?;
    match std::fs::symlink_metadata(&child) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            require_contained_real_directory(root, &child, "evidence child")?;
            std::fs::remove_dir_all(&child)
                .with_context(|| format!("removing {}", child.display()))?;
        }
        Ok(_) => {
            std::fs::remove_file(&child).with_context(|| {
                format!("removing unexpected evidence entry {}", child.display())
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", child.display()));
        }
    }
    Ok(())
}

fn checked_child(root: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        matches!(
            name,
            "work-a"
                | "work-b"
                | "path-a"
                | "path-b"
                | "qualification"
                | "toolchain-b"
                | "cargo-home-b"
        ),
        "refusing to modify unexpected evidence child `{name}`"
    );
    let child = root.join(name);
    ensure!(
        child.parent() == Some(root),
        "evidence child escaped {}",
        root.display()
    );
    Ok(child)
}

fn write_manifest(root: &Path, manifest: &Path, text: &str) -> Result<()> {
    let temporary = root.join(MANIFEST_TEMP);
    require_evidence_root(root)?;
    ensure!(
        manifest == root.join(MANIFEST_NAME) && temporary.parent() == Some(root),
        "evidence manifest escaped its root"
    );
    let mut options = OpenOptions::new();
    OpenOptions::create_new(&mut options, true);
    OpenOptions::write(&mut options, true);
    let mut file = OpenOptions::open(&options, &temporary)
        .with_context(|| format!("creating {}", temporary.display()))?;
    file.write_all(text.as_bytes())
        .with_context(|| format!("writing {}", temporary.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", temporary.display()))?;
    require_contained_regular_file(root, &temporary, "evidence staging manifest")?;
    std::fs::rename(&temporary, manifest).with_context(|| {
        format!(
            "committing {} as {}",
            temporary.display(),
            manifest.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn parked_qemu_version_fixture() {
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn a_parked_qemu_identity_probe_reaches_its_named_deadline() {
        let executable = std::env::current_exe().unwrap();
        let root = std::env::current_dir().unwrap();
        let error = qemu_version_output_with(&root, &executable, |command, deadline, label| {
            assert_eq!(command.get_program(), executable.as_os_str());
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                [std::ffi::OsStr::new("--version")]
            );
            assert_eq!(command.get_current_dir(), Some(root.as_path()));
            assert_eq!(deadline, QEMU_VERSION_DEADLINE);
            assert_eq!(label, QEMU_VERSION_LABEL);

            let mut parked = Command::new(&executable);
            parked.args([
                "--exact",
                "evidence::tests::parked_qemu_version_fixture",
                "--ignored",
            ]);
            bounded_process::output(parked, Duration::from_millis(50), label)
        })
        .unwrap_err();

        assert!(format!("{error:#}").contains("QEMU version identity probe timed out after 50 ms"));
    }

    #[test]
    fn a_dirty_source_cannot_be_called_clean() {
        assert!(status_is_clean("\r\n  "));
        assert!(!status_is_clean(" M kernel/src/main.rs\n"));
        assert!(!status_is_clean("?? untracked.rs\n"));
    }

    #[test]
    fn release_source_capture_requires_two_clean_views_of_one_identity() {
        let current = ("a".repeat(40), "b".repeat(40));
        require_stable_clean_source("", &current, "", &current).unwrap();

        let changed = ("9".repeat(40), current.1.clone());
        assert!(require_stable_clean_source("", &current, "", &changed).is_err());
        assert!(require_stable_clean_source(" M Cargo.lock\n", &current, "", &current).is_err());
        assert!(
            require_stable_clean_source("", &current, "?? transient-input\n", &current).is_err()
        );
    }

    #[test]
    fn release_reproduction_uses_copied_sources_and_rejects_host_paths() {
        let implementation = include_str!("evidence.rs");
        let create = implementation.find("pub(crate) fn create(").unwrap();
        let verify = implementation[create..]
            .find("pub(crate) fn verify(")
            .unwrap()
            + create;
        let body = &implementation[create..verify];
        assert_eq!(body.matches("reproduce_release(paths,").count(), 1);
        assert_eq!(
            body.matches("reproduce_release_alternate(paths,").count(),
            1
        );
        assert!(body.contains(".zip(&release_b.host_roots)"));
        assert!(body.contains(".all(|(left, right)| left != right)"));
        assert!(body.contains("require_no_host_paths(path, maximum, label, &host_roots)?"));

        let reproduce = implementation.find("fn reproduce_release(").unwrap();
        let absent = implementation[reproduce..]
            .find("fn require_no_host_paths(")
            .unwrap()
            + reproduce;
        assert!(
            implementation[reproduce..absent]
                .contains("mutation_workspace::with_clean_source_copy(paths, build)")
        );
        assert_eq!(
            implementation[reproduce..absent]
                .matches("mutation_workspace::with_clean_source_copy(paths")
                .count(),
            2
        );
        assert!(implementation[reproduce..absent].contains("CARGO_NET_OFFLINE"));
        assert!(implementation[reproduce..absent].contains("std::fs::hard_link"));
        assert!(!implementation[reproduce..absent].contains("credentials.toml"));

        let root =
            std::env::temp_dir().join(format!("wizard-host-path-evidence-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let artifact = root.join("artifact");
        let host = PathBuf::from(r"C:\private host");
        std::fs::write(&artifact, b"prefix C:\\private host suffix").unwrap();
        assert!(
            require_no_host_paths(&artifact, 1024, "fixture", std::slice::from_ref(&host)).is_err()
        );
        std::fs::write(&artifact, b"prefix /wizard/source suffix").unwrap();
        require_no_host_paths(&artifact, 1024, "fixture", &[host]).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parent_docs_relation_accepts_only_exact_regular_file_modifications() {
        let old = "1".repeat(40);
        let new = "2".repeat(40);
        let raw = format!(":100644 100644 {old} {new} M\0docs/016-pre-wizzy-foundation.md\0");
        let qualified = qualification::SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        assert_eq!(
            QualificationRelation::from_parent_diff(&qualified, raw.as_bytes())
                .unwrap()
                .files,
            ["docs/016-pre-wizzy-foundation.md"]
        );
        let changed_blob = format!(
            ":100644 100644 {old} {} M\0docs/016-pre-wizzy-foundation.md\0",
            "3".repeat(40)
        );
        assert_ne!(
            QualificationRelation::from_parent_diff(&qualified, raw.as_bytes())
                .unwrap()
                .digest,
            QualificationRelation::from_parent_diff(&qualified, changed_blob.as_bytes())
                .unwrap()
                .digest,
        );

        let added = format!(":000000 100644 {} {new} A\0README.md\0", "0".repeat(40));
        assert!(parse_parent_docs_diff(added.as_bytes()).is_err());
        let source = format!(":100644 100644 {old} {new} M\0xtask/src/evidence.rs\0");
        assert!(parse_parent_docs_diff(source.as_bytes()).is_err());
        let empty = b"";
        assert!(parse_parent_docs_diff(empty).is_err());
    }

    #[test]
    fn the_parent_diff_admits_only_sorted_unique_regular_modifications() {
        let old = "1".repeat(40);
        let new = "2".repeat(40);
        let row = |modes: &str, status: &str, path: &str| {
            format!(":{modes} {old} {new} {status}\0{path}\0")
        };
        let refused = |raw: &str, diagnostic: &str| {
            let error = parse_parent_docs_diff(raw.as_bytes()).unwrap_err();
            assert!(format!("{error:#}").contains(diagnostic), "{error:#}");
        };
        let readme = row("100644 100644", "M", "README.md");
        let log = row("100644 100644", "M", "docs/004-decision-log.md");
        assert_eq!(
            parse_parent_docs_diff(format!("{readme}{log}").as_bytes()).unwrap(),
            ["README.md", "docs/004-decision-log.md"]
        );
        refused(&format!("{log}{readme}"), "not unique and sorted");
        refused(&format!("{readme}{readme}"), "not unique and sorted");
        refused(
            &row("100644 100644", "T", "README.md"),
            "ordinary 100644 file modifications",
        );
        refused(
            &row("100755 100755", "M", "README.md"),
            "ordinary 100644 file modifications",
        );
        refused(readme.trim_end_matches('\0'), "not NUL terminated");
        refused(&format!("{readme}extra\0"), "incomplete row");
    }

    #[test]
    fn same_commit_relation_requires_the_live_current_mutation_pass() {
        assert!(relation_requires_current_mutation("same-commit"));
        assert!(!relation_requires_current_mutation("parent-docs-only-v1"));
    }

    #[test]
    fn parent_docs_relation_requires_one_exact_direct_parent() {
        let evidence = "1".repeat(40);
        let qualified = "2".repeat(40);
        require_single_direct_parent(&format!("{evidence} {qualified}\n"), &evidence, &qualified)
            .unwrap();
        assert!(
            require_single_direct_parent(
                &format!("{evidence} {}\n", "3".repeat(40)),
                &evidence,
                &qualified
            )
            .is_err()
        );
        assert!(
            require_single_direct_parent(
                &format!("{evidence} {qualified} {}\n", "3".repeat(40)),
                &evidence,
                &qualified
            )
            .is_err()
        );
    }

    #[test]
    fn a_duplicate_evidence_key_is_rejected_before_canonical_rendering() {
        let mut record = Record::default();
        record.insert("source.commit", "one").unwrap();

        let error = record.insert("source.commit", "two").unwrap_err();
        assert_eq!(error.to_string(), "duplicate evidence key `source.commit`");
    }

    #[test]
    fn an_oversized_manifest_is_refused_before_payload_allocation() {
        let root =
            std::env::temp_dir().join(format!("wizard-oversized-manifest-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let paths = Paths::new(root.join("repository"), Some(&root.join("target")));
        let evidence = evidence_root(&paths);
        std::fs::create_dir_all(&evidence).unwrap();
        File::create(evidence.join(MANIFEST_NAME))
            .unwrap()
            .set_len(MAX_MANIFEST_BYTES + 1)
            .unwrap();

        let error = verify(&paths).unwrap_err().to_string();
        assert!(error.contains("exceeding the 1048576-byte allocation limit"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_evidence_digests_have_a_streaming_ceiling() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-digest-bound-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let path = root.join("artifact");
        File::create(&path).unwrap().set_len(33).unwrap();

        let error = digest_file_bounded(&path, 32, "fixture artifact")
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("exceeding its 32-byte hashing limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_evidence_digest_growth_is_refused_after_measurement() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-digest-growth-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let path = root.join("artifact");
        std::fs::write(&path, b"x").unwrap();
        let growing = path.clone();

        let error = digest_file_bounded_with(
            &path,
            4,
            "fixture artifact",
            move || {
                let mut options = OpenOptions::new();
                OpenOptions::write(&mut options, true);
                OpenOptions::open(&options, growing)?.set_len(5)?;
                Ok(())
            },
            || Ok(()),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("grew beyond its 4-byte hashing limit while it was hashed"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_evidence_digest_truncation_is_refused_during_hashing() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-digest-truncation-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let path = root.join("artifact");
        std::fs::write(&path, b"1234").unwrap();
        let shrinking = path.clone();

        let error = digest_file_bounded_with(
            &path,
            4,
            "fixture artifact",
            move || {
                let mut options = OpenOptions::new();
                OpenOptions::write(&mut options, true);
                OpenOptions::open(&options, shrinking)?.set_len(3)?;
                Ok(())
            },
            || Ok(()),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("changed length while it was hashed"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_evidence_digest_remeasures_after_hashing() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-digest-remeasure-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let path = root.join("artifact");
        std::fs::write(&path, b"1234").unwrap();
        let shrinking = path.clone();

        let error = digest_file_bounded_with(
            &path,
            4,
            "fixture artifact",
            || Ok(()),
            move || {
                let mut options = OpenOptions::new();
                OpenOptions::write(&mut options, true);
                OpenOptions::open(&options, shrinking)?.set_len(3)?;
                Ok(())
            },
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("changed length after it was hashed"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deterministic_artifact_size_is_checked_before_hashing_payload_bytes() {
        let root = std::env::temp_dir().join(format!(
            "wizard-exact-evidence-artifact-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let path = root.join("oversized.img");
        File::create(&path).unwrap().set_len(33).unwrap();
        let reached = std::cell::Cell::new(false);

        let error = digest_file_exact_with(&path, 32, "fixture image", |_file| {
            reached.set(true);
            Ok(Digest {
                bytes: 33,
                sha256: "0".repeat(64),
            })
        })
        .unwrap_err()
        .to_string();

        assert!(error.contains("33 bytes, expected exactly 32"), "{error}");
        assert!(!reached.get(), "oversized image bytes reached the hasher");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deterministic_artifact_digest_requires_the_exact_returned_byte_count() {
        let root = std::env::temp_dir().join(format!(
            "wizard-exact-evidence-count-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let path = root.join("image");
        File::create(&path).unwrap().set_len(32).unwrap();

        let error = digest_file_exact_with(&path, 32, "fixture image", |_file| {
            Ok(Digest {
                bytes: 33,
                sha256: "0".repeat(64),
            })
        })
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("fixture image hashed 33 bytes, expected exactly 32"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deterministic_artifact_digest_remeasures_the_exact_extent() {
        let root = std::env::temp_dir().join(format!(
            "wizard-exact-evidence-remeasure-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let path = root.join("image");
        File::create(&path).unwrap().set_len(32).unwrap();
        let growing = path.clone();

        let error = digest_file_exact_with(&path, 32, "fixture image", move |file| {
            let (bytes, digest) = sha256::digest_reader(file)?;
            let mut options = OpenOptions::new();
            OpenOptions::write(&mut options, true);
            OpenOptions::open(&options, growing)?.set_len(33)?;
            Ok(Digest {
                bytes,
                sha256: sha256::hex(&digest),
            })
        })
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("changed outside its exact 32-byte extent while being hashed"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_manifest_schema_is_exact() {
        let mut record = Record::default();
        record.insert("known", "value").unwrap();
        let expected = BTreeSet::from(["known".to_string(), "required".to_string()]);
        assert!(require_exact_keys(&record, &expected).is_err());

        record.insert("required", "value").unwrap();
        record.insert("unknown", "value").unwrap();
        assert!(require_exact_keys(&record, &expected).is_err());
    }

    #[test]
    fn equal_lengths_do_not_make_artifacts_identical() {
        let first = Digest {
            bytes: 4096,
            sha256: "a".repeat(64),
        };
        let second = Digest {
            bytes: 4096,
            sha256: "b".repeat(64),
        };
        assert!(!byte_identical(&first, &second));
        assert!(byte_identical(&first, &first));
    }

    #[test]
    fn every_recorded_result_must_pass() {
        let mut record = Record::default();
        for step in gate::REGISTRY.steps() {
            record.insert(step.evidence_key(), PASS).unwrap();
        }
        record
            .insert("result.artifacts-byte-identical", PASS)
            .unwrap();
        record
            .insert("result.shipped-artifact-boot", "fail")
            .unwrap();
        assert!(require_pass_results(&record).is_err());
    }

    #[test]
    fn rendering_is_sorted_and_round_trips() {
        let mut record = Record::default();
        record.insert("z.last", "two").unwrap();
        record.insert("a.first", "one").unwrap();
        let rendered = record.render();
        assert!(rendered.starts_with(&format!("{FORMAT}\na.first=one\nz.last=two\n")));
        assert_eq!(parse_manifest(&rendered).unwrap(), record);
        assert!(parse_manifest(rendered.trim_end()).is_err());
        assert!(parse_manifest(&format!("{FORMAT}\nz.last=two\na.first=one\n")).is_err());
    }

    #[test]
    fn a_broken_manifest_symlink_cannot_redirect_the_writer() {
        let root =
            std::env::temp_dir().join(format!("wizard-evidence-symlink-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let paths = Paths::new(root.join("repo"), Some(&root.join("target")));
        let evidence = evidence_root(&paths);
        let external = root.join("outside-manifest");
        std::fs::create_dir_all(&evidence).unwrap();
        create_file_symlink(&external, &evidence.join(MANIFEST_TEMP));

        invalidate_manifest(&paths).unwrap();

        assert!(!std::fs::symlink_metadata(evidence.join(MANIFEST_TEMP)).is_ok());
        assert!(!external.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating broken manifest symlink");
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("creating broken manifest symlink");
    }

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating directory symlink");
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_dir(target, link).expect("creating directory symlink");
    }

    fn write_qualification_bundle_fixture(root: &Path) {
        for spec in qualification::BUNDLE_FILES {
            let path = root.join(spec.relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut file = File::create(path).unwrap();
            file.write_all(spec.prefix.as_bytes()).unwrap();
            let (maximum, exact) = qualification::bundle_file_size_policy(spec).unwrap();
            if exact {
                file.set_len(maximum).unwrap();
            }
        }
    }

    #[test]
    fn a_substituted_evidence_root_is_never_recursively_reset() {
        let root =
            std::env::temp_dir().join(format!("wizard-evidence-root-link-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let outside = root.join("outside");
        let evidence = root.join("evidence-link");
        std::fs::create_dir_all(outside.join("work-a")).unwrap();
        std::fs::write(outside.join("work-a/keep"), "outside").unwrap();
        create_directory_symlink(&outside, &evidence);

        assert!(reset_child(&evidence, "work-a").is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("work-a/keep")).unwrap(),
            "outside"
        );

        let ancestor_target = root.join("ancestor-target");
        let ancestor_link = root.join("ancestor-link");
        std::fs::create_dir_all(ancestor_target.join("evidence")).unwrap();
        create_directory_symlink(&ancestor_target, &ancestor_link);
        assert!(require_evidence_root(&ancestor_link.join("evidence")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_redirected_target_is_refused_before_evidence_root_creation() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-target-link-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let outside = root.join("outside");
        let target = root.join("target-link");
        std::fs::create_dir_all(&outside).unwrap();
        create_directory_symlink(&outside, &target);
        let paths = Paths::new(root.join("repository"), Some(&target));

        assert!(invalidate_manifest(&paths).is_err());
        assert!(!outside.join("evidence").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_bytes_must_match_the_manifest() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-artifact-fixture-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        for spec in ARTIFACTS {
            let path = root.join(spec.relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let bytes = if spec.prefix.ends_with("kernel") {
                b"same kernel".as_slice()
            } else {
                b"same image".as_slice()
            };
            std::fs::write(path, bytes).unwrap();
        }

        let mut record = Record::default();
        add_artifacts_with_image_bytes(&mut record, &root, b"same image".len() as u64).unwrap();
        verify_artifacts_with_image_bytes(&record, &root, b"same image".len() as u64).unwrap();
        assert!(
            verify_artifacts(&record, &root).is_err(),
            "an undersized release image passed the production size policy"
        );
        std::fs::write(root.join(ARTIFACTS[1].relative), b"evil image").unwrap();
        std::fs::write(root.join(ARTIFACTS[3].relative), b"evil image").unwrap();
        assert!(
            verify_artifacts_with_image_bytes(&record, &root, b"same image".len() as u64).is_err()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_evidence_attempt_invalidates_an_older_success_first() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-invalidation-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let paths = Paths::new(root.join("repo"), Some(&root.join("target")));
        let evidence = evidence_root(&paths);
        std::fs::create_dir_all(&evidence).unwrap();
        std::fs::write(evidence.join(MANIFEST_NAME), "stale pass").unwrap();

        invalidate_manifest(&paths).unwrap();

        assert!(!evidence.join(MANIFEST_NAME).exists());
        let source = include_str!("evidence.rs");
        let create = source.find("pub(crate) fn create").unwrap();
        let verify = source[create..].find("pub(crate) fn verify").unwrap() + create;
        let body = &source[create..verify];
        assert!(
            body.find("invalidate_manifest(paths)?;").unwrap()
                < body.find("qemu::nvme_image").unwrap()
        );
        assert!(
            body.find("verify_record(paths, &root, &record)?;").unwrap()
                < body
                    .find("write_manifest(&root, &manifest, &record.render())?;")
                    .unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn qualification_records_cannot_be_omitted_or_changed_offline() {
        let bundled: BTreeSet<&str> = qualification::BUNDLE_FILES
            .iter()
            .map(|spec| spec.relative)
            .collect();
        assert_eq!(
            bundled,
            BTreeSet::from([
                "qualification/cargo-mutants-grade/adjudications.json",
                "qualification/cargo-mutants-grade/doctest.transcript",
                "qualification/cargo-mutants-grade/inventory.json",
                "qualification/cargo-mutants-grade/mutants.json",
                "qualification/cargo-mutants-grade/outcomes.json",
                "qualification/cargo-mutants-grade/record",
                "qualification/cargo-mutants-grade/request",
                "qualification/codex-security-deep/coverage.json",
                "qualification/codex-security-deep/findings.json",
                "qualification/codex-security-deep/record",
                "qualification/codex-security-deep/report.md",
                "qualification/codex-security-deep/request",
                "qualification/codex-security-deep/scan-manifest.json",
                "qualification/advisory/.cargo/audit.toml",
                "qualification/advisory/cargo-audit-refresh.transcript",
                "qualification/advisory/cargo-audit.transcript",
                "qualification/advisory/record",
                "qualification/model/record",
                "qualification/model/ring-clean.transcript",
                "qualification/model/ring-memory-clean.transcript",
                "qualification/model/ring-memory-duplicate-terminal.transcript",
                "qualification/model/ring-memory-load-relaxed.transcript",
                "qualification/model/ring-memory-no-reservation.transcript",
                "qualification/model/ring-memory-store-relaxed.transcript",
                "qualification/mutation/record",
                "qualification/mutation/report",
                "qualification/vmware-self-test/generation.receipt",
                "qualification/vmware-self-test/record",
                "qualification/vmware-self-test/serial.log",
                "qualification/vmware-self-test/vmrun-list.txt",
                "qualification/vmware-self-test/wizard-nvm-flat.img",
                "qualification/vmware-self-test/wizard-nvm.vmdk",
                "qualification/vmware-self-test/bedrock-flat.img",
                "qualification/vmware-self-test/bedrock.vmdk",
                "qualification/vmware-self-test/bedrock.vmx",
                "qualification/vmware-self-test/dma-handoff.bin",
                "qualification/vmware-shipped/generation.receipt",
                "qualification/vmware-shipped/record",
                "qualification/vmware-shipped/serial.log",
                "qualification/vmware-shipped/vmrun-list.txt",
                "qualification/vmware-shipped/wizard-nvm-flat.img",
                "qualification/vmware-shipped/wizard-nvm.vmdk",
                "qualification/vmware-shipped/bedrock-flat.img",
                "qualification/vmware-shipped/bedrock.vmdk",
                "qualification/vmware-shipped/bedrock.vmx",
                "qualification/vmware-shipped/dma-handoff.bin",
            ])
        );
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-qualification-fixture-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        write_qualification_bundle_fixture(&root);
        let mut record = Record::default();
        add_qualification_files(&mut record, &root).unwrap();
        let expected: BTreeSet<String> = record.fields.keys().cloned().collect();

        record.fields.remove("qualification.model.record.sha256");
        assert!(require_exact_keys(&record, &expected).is_err());

        let mut record = Record::default();
        add_qualification_files(&mut record, &root).unwrap();
        let model_record = root.join("qualification/model/record");
        let bytes = std::fs::read(&model_record).unwrap();
        std::fs::write(&model_record, vec![b'x'; bytes.len()]).unwrap();
        let error = verify_qualification_files(&record, &root)
            .unwrap_err()
            .to_string();
        assert!(error.contains("qualification.model.record"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_bundle_parent_link_is_rejected_before_qualification_hashing() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-parent-link-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let spec = qualification::BUNDLE_FILES
            .iter()
            .find(|spec| spec.relative == "qualification/model/record")
            .unwrap();
        let model = root.join("qualification/model");
        let alternate = root.join("qualification/model-alternate");
        std::fs::create_dir_all(&model).unwrap();
        std::fs::write(model.join("record"), spec.prefix).unwrap();
        std::fs::rename(&model, &alternate).unwrap();
        create_directory_symlink(&alternate, &model);
        let hashing_started = std::cell::Cell::new(false);

        assert!(
            verified_qualification_digest_with(&root, spec, || {
                hashing_started.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(!hashing_started.get());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exceptional_artifacts_are_the_exact_reproduced_inputs() {
        let root = std::env::temp_dir().join(format!(
            "wizard-evidence-cross-artifact-fixture-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let repository = root.join("repo");
        let evidence = root.join("evidence");
        let paths = Paths::new(repository.clone(), None);
        let files = [
            evidence.join(ARTIFACTS[1].relative),
            evidence.join("qualification/vmware-self-test/bedrock-flat.img"),
            evidence.join("qualification/vmware-shipped/bedrock-flat.img"),
            evidence.join("qualification/vmware-self-test/wizard-nvm-flat.img"),
            evidence.join("qualification/vmware-shipped/wizard-nvm-flat.img"),
            qemu::nvme_path(&repository),
        ];
        for path in &files {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(&files[0], "shipped!").unwrap();
        std::fs::write(&files[1], "selftest").unwrap();
        std::fs::write(&files[2], "shipped!").unwrap();
        for path in &files[3..] {
            std::fs::write(path, "namespace").unwrap();
        }
        assert!(verify_qualification_artifact_identity_with_sizes(&paths, &evidence, 8, 9).is_ok());

        std::fs::write(&files[2], "changed!").unwrap();
        assert!(
            verify_qualification_artifact_identity_with_sizes(&paths, &evidence, 8, 9).is_err()
        );
        std::fs::write(&files[2], "shipped!").unwrap();
        std::fs::write(&files[4], "foreign!!").unwrap();
        assert!(
            verify_qualification_artifact_identity_with_sizes(&paths, &evidence, 8, 9).is_err()
        );
        std::fs::write(&files[4], "namespace").unwrap();
        std::fs::write(&files[1], "shipped!").unwrap();
        assert!(
            verify_qualification_artifact_identity_with_sizes(&paths, &evidence, 8, 9).is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn optional_tools_never_change_the_manifest_schema() {
        let mut absent = Record::default();
        add_tool_identity(&mut absent, "tool.java", &ToolIdentity::Unavailable).unwrap();

        let mut present = Record::default();
        add_tool_identity(
            &mut present,
            "tool.java",
            &ToolIdentity::Available {
                version: "java 21\nVM build 11".to_string(),
                digest: Digest {
                    bytes: 42,
                    sha256: "a".repeat(64),
                },
            },
        )
        .unwrap();

        assert_eq!(
            absent.fields.keys().collect::<Vec<_>>(),
            present.fields.keys().collect::<Vec<_>>()
        );
        assert_eq!(absent.get("tool.java.state").unwrap(), UNAVAILABLE);
        assert_eq!(
            present.get("tool.java.version").unwrap(),
            "java 21\\nVM build 11"
        );
    }

    #[test]
    fn an_unavailable_qualification_dependency_is_named_exactly() {
        let mut recorded = Record::default();
        let mut observed = Record::default();
        add_tool_identity(
            &mut recorded,
            "tool.java",
            &ToolIdentity::Available {
                version: "java 21".to_string(),
                digest: Digest {
                    bytes: 42,
                    sha256: "a".repeat(64),
                },
            },
        )
        .unwrap();
        add_tool_identity(&mut observed, "tool.java", &ToolIdentity::Unavailable).unwrap();
        add_tool_identity(&mut recorded, "tool.vmware", &ToolIdentity::Unavailable).unwrap();
        add_tool_identity(&mut observed, "tool.vmware", &ToolIdentity::Unavailable).unwrap();

        let error = require_inputs_match(&recorded, &observed)
            .unwrap_err()
            .to_string();
        assert!(error.contains("tool.java"), "{error}");
        assert!(error.contains("manifest `available`"), "{error}");
        assert!(error.contains("current `unavailable`"), "{error}");
    }
}
