//! Reproducible release evidence and its offline verifier.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::cargo::Profile;
use crate::paths::Paths;
use crate::{gate, image, qemu, sha256, tasks, vendor};

const FORMAT: &str = "wizard-evidence-v1";
const MANIFEST_NAME: &str = "MANIFEST";
const MANIFEST_TEMP: &str = "MANIFEST.incomplete";
const PASS: &str = "pass";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Digest {
    bytes: u64,
    sha256: String,
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
        relative: "path-a/wizard-os.img",
    },
    ArtifactSpec {
        prefix: "artifact.path-b.kernel",
        relative: "path-b/wizard-kernel",
    },
    ArtifactSpec {
        prefix: "artifact.path-b.image",
        relative: "path-b/wizard-os.img",
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Record {
    fields: BTreeMap<String, String>,
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
    println!("[xtask] qualifying a reproducible release from a clean checkout");
    qemu::nvme_image(paths.root()).context("materialising the deterministic NVMe input")?;
    vendor::verify(paths)?;
    let before = input_record(paths)?;

    tasks::ci(paths).context("release evidence stopped at the local gate")?;
    ensure!(
        input_record(paths)? == before,
        "source, tools, firmware or boot inputs changed during qualification"
    );

    let root = evidence_root(paths);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating evidence directory {}", root.display()))?;
    for child in ["work-a", "work-b", "path-a", "path-b"] {
        reset_child(&root, child)?;
    }

    let work_a = root.join("work-a");
    let work_b = root.join("work-b");
    let paths_a = Paths::new(paths.root().to_path_buf(), Some(&work_a));
    let paths_b = Paths::new(paths.root().to_path_buf(), Some(&work_b));
    let image_a = image::build(&paths_a, Profile::Release, image::Kernel::Shipped)
        .context("building clean release path A")?;
    let image_b = image::build(&paths_b, Profile::Release, image::Kernel::Shipped)
        .context("building clean release path B")?;
    let kernel_a = paths_a.kernel_elf(Profile::Release);
    let kernel_b = paths_b.kernel_elf(Profile::Release);

    let built_kernel_a = digest_file(&kernel_a)?;
    let built_kernel_b = digest_file(&kernel_b)?;
    let built_image_a = digest_file(&image_a)?;
    let built_image_b = digest_file(&image_b)?;
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
    copy_exact(&kernel_a, &bundled_kernel_a, &built_kernel_a)?;
    copy_exact(&image_a, &bundled_image_a, &built_image_a)?;
    copy_exact(&kernel_b, &bundled_kernel_b, &built_kernel_b)?;
    copy_exact(&image_b, &bundled_image_b, &built_image_b)?;

    tasks::boot_shipped_image(&paths_a, &bundled_image_a)
        .context("booting the exact path-A evidence image")?;
    ensure!(
        digest_file(&bundled_image_a)? == built_image_a,
        "the boot changed the release image it qualified"
    );

    remove_child(&root, "work-a")?;
    remove_child(&root, "work-b")?;

    let mut record = input_record(paths)?;
    ensure!(
        record == before,
        "source, tools, firmware or boot inputs changed during artifact reproduction"
    );
    add_artifacts(&mut record, &root)?;
    for step in gate::REGISTRY.steps() {
        record.insert(step.evidence_key(), PASS)?;
    }
    record.insert("result.artifacts-byte-identical", PASS)?;
    record.insert("result.shipped-artifact-boot", PASS)?;

    let manifest = root.join(MANIFEST_NAME);
    write_manifest(&root, &manifest, &record.render())?;
    verify(paths)?;
    println!("[xtask] release evidence ready at {}", manifest.display());
    Ok(())
}

/// Verify the evidence bundle against the current clean checkout and installed
/// tools without contacting a network or rebuilding anything.
pub(crate) fn verify(paths: &Paths) -> Result<()> {
    let root = evidence_root(paths);
    let manifest = root.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("reading evidence manifest {}", manifest.display()))?;
    let record = parse_manifest(&text)?;

    vendor::verify(paths)?;
    let observed = input_record(paths)?;
    let mut expected_keys: BTreeSet<String> = observed.fields.keys().cloned().collect();
    for spec in ARTIFACTS {
        expected_keys.insert(format!("{}.path", spec.prefix));
        expected_keys.insert(format!("{}.bytes", spec.prefix));
        expected_keys.insert(format!("{}.sha256", spec.prefix));
    }
    for step in gate::REGISTRY.steps() {
        expected_keys.insert(step.evidence_key().to_string());
    }
    expected_keys.insert("result.artifacts-byte-identical".to_string());
    expected_keys.insert("result.shipped-artifact-boot".to_string());
    require_exact_keys(&record, &expected_keys)?;

    for (key, value) in &observed.fields {
        ensure!(
            record.get(key)? == value,
            "evidence input `{key}` differs from the manifest"
        );
    }
    verify_artifacts(&record, &root)?;
    require_pass_results(&record)?;
    println!("[xtask] evidence manifest and every recorded byte verify offline");
    Ok(())
}

fn input_record(paths: &Paths) -> Result<Record> {
    let mut record = Record::default();
    let (commit, tree) = clean_source(paths.root())?;
    record.insert("source.commit", commit)?;
    record.insert("source.tree", tree)?;
    add_digest(
        &mut record,
        "source.cargo-lock",
        &paths.root().join("Cargo.lock"),
    )?;
    add_digest(
        &mut record,
        "source.rust-toolchain",
        &paths.root().join("rust-toolchain.toml"),
    )?;

    record.insert(
        "tool.rustc.version",
        version_output(paths.root(), Path::new("rustc"), &["-vV"])
            .context("reading the Rust compiler version")?,
    )?;
    record.insert(
        "tool.cargo.version",
        version_output(paths.root(), Path::new(env!("CARGO")), &["-Vv"])
            .context("reading the Cargo version")?,
    )?;
    let qemu = qemu::installation()?;
    record.insert(
        "tool.qemu.version",
        version_output(paths.root(), &qemu.emulator, &["--version"])
            .context("reading the QEMU version")?,
    )?;
    add_digest(&mut record, "tool.qemu", &qemu.emulator)?;
    add_digest(&mut record, "firmware.code", &qemu.code_firmware)?;
    add_digest(&mut record, "firmware.vars-template", &qemu.vars_firmware)?;
    add_digest(&mut record, "input.nvme", &qemu::nvme_path(paths.root()))?;
    add_vendor_inputs(&mut record, paths)?;
    Ok(record)
}

fn add_vendor_inputs(record: &mut Record, paths: &Paths) -> Result<()> {
    for (label, listing) in [
        ("limine", paths.limine_checksums()),
        ("tla", paths.tla_checksums()),
        ("fat32-corpus", paths.fat32_corpus_checksums()),
    ] {
        add_digest(record, &format!("vendor.{label}.listing"), &listing)?;
        let text = std::fs::read_to_string(&listing)
            .with_context(|| format!("reading {}", listing.display()))?;
        let entries =
            vendor::parse(&text).with_context(|| format!("parsing {}", listing.display()))?;
        let directory = listing
            .parent()
            .context("vendor listing has no directory")?;
        for (index, entry) in entries.iter().enumerate() {
            let prefix = format!("vendor.{label}.payload.{:02}", index + 1);
            record.insert(format!("{prefix}.name"), entry.name.clone())?;
            add_digest(record, &prefix, &directory.join(&entry.name))?;
        }
    }
    Ok(())
}

fn add_artifacts(record: &mut Record, root: &Path) -> Result<()> {
    for spec in ARTIFACTS {
        record.insert(format!("{}.path", spec.prefix), spec.relative)?;
        add_digest(record, spec.prefix, &root.join(spec.relative))?;
    }
    Ok(())
}

fn verify_artifacts(record: &Record, root: &Path) -> Result<()> {
    let mut digests = Vec::new();
    for spec in ARTIFACTS {
        ensure!(
            record.get(&format!("{}.path", spec.prefix))? == spec.relative,
            "{} names an unexpected bundle path",
            spec.prefix
        );
        let digest = digest_file(&root.join(spec.relative))?;
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
        lines.next() == Some(FORMAT),
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
    let status = git_output(root, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    ensure!(
        status_is_clean(&status),
        "release evidence requires a clean checkout:\n{status}"
    );
    let commit = git_output(root, &["rev-parse", "HEAD"])?;
    let tree = git_output(root, &["rev-parse", "HEAD^{tree}"])?;
    ensure!(is_git_id(&commit), "Git returned an invalid commit ID");
    ensure!(is_git_id(&tree), "Git returned an invalid tree ID");
    Ok((commit, tree))
}

fn status_is_clean(status: &str) -> bool {
    status.trim().is_empty()
}

fn is_git_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_output(root: &Path, args: &[&str]) -> Result<String> {
    command_output(root, Path::new("git"), args)
        .with_context(|| format!("running git {}", args.join(" ")))
}

fn command_output(root: &Path, program: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .current_dir(root)
        .args(args)
        .output()
        .with_context(|| format!("launching {}", program.display()))?;
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

fn version_output(root: &Path, program: &Path, args: &[&str]) -> Result<String> {
    let output = command_output(root, program, args)?;
    ensure!(
        !output.is_empty(),
        "{} printed no version",
        program.display()
    );
    Ok(output.replace('\n', "\\n"))
}

fn add_digest(record: &mut Record, prefix: &str, path: &Path) -> Result<()> {
    let digest = digest_file(path)?;
    record.insert(format!("{prefix}.bytes"), digest.bytes.to_string())?;
    record.insert(format!("{prefix}.sha256"), digest.sha256)?;
    Ok(())
}

fn digest_file(path: &Path) -> Result<Digest> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Digest {
        bytes: bytes.len() as u64,
        sha256: sha256::hex(&sha256::digest(&bytes)),
    })
}

fn byte_identical(first: &Digest, second: &Digest) -> bool {
    first.bytes == second.bytes && first.sha256 == second.sha256
}

fn copy_exact(source: &Path, destination: &Path, expected: &Digest) -> Result<()> {
    std::fs::copy(source, destination)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    ensure!(
        digest_file(destination)? == *expected,
        "{} changed while entering the evidence bundle",
        source.display()
    );
    Ok(())
}

fn evidence_root(paths: &Paths) -> PathBuf {
    paths.target_dir().join("evidence")
}

fn reset_child(root: &Path, name: &str) -> Result<()> {
    remove_child(root, name)?;
    let child = checked_child(root, name)?;
    std::fs::create_dir_all(&child).with_context(|| format!("creating {}", child.display()))
}

fn remove_child(root: &Path, name: &str) -> Result<()> {
    let child = checked_child(root, name)?;
    if child.exists() {
        std::fs::remove_dir_all(&child).with_context(|| format!("removing {}", child.display()))?;
    }
    Ok(())
}

fn checked_child(root: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        matches!(name, "work-a" | "work-b" | "path-a" | "path-b"),
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
    std::fs::write(&temporary, text).with_context(|| format!("writing {}", temporary.display()))?;
    if manifest.exists() {
        std::fs::remove_file(manifest)
            .with_context(|| format!("replacing {}", manifest.display()))?;
    }
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
    fn a_dirty_source_cannot_be_called_clean() {
        assert!(status_is_clean("\r\n  "));
        assert!(!status_is_clean(" M kernel/src/main.rs\n"));
        assert!(!status_is_clean("?? untracked.rs\n"));
    }

    #[test]
    fn a_manifest_rejects_duplicate_keys() {
        let text = format!("{FORMAT}\nsource.commit=one\nsource.commit=two\n");
        assert!(parse_manifest(&text).is_err());
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
        add_artifacts(&mut record, &root).unwrap();
        std::fs::write(root.join(ARTIFACTS[1].relative), b"evil image").unwrap();
        std::fs::write(root.join(ARTIFACTS[3].relative), b"evil image").unwrap();
        assert!(verify_artifacts(&record, &root).is_err());

        std::fs::remove_dir_all(root).unwrap();
    }
}
