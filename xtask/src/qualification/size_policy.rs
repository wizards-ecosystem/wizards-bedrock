use super::*;

pub(super) fn require_artifact_size(kind: Kind, name: &str, path: &Path) -> Result<()> {
    let length = File::open(path)
        .with_context(|| format!("opening qualification artifact {}", path.display()))?
        .metadata()
        .with_context(|| format!("measuring qualification artifact {}", path.display()))?
        .len();
    let (maximum, exact) = artifact_size_policy(kind, name)?;
    if exact {
        ensure!(
            length == maximum,
            "qualification artifact {name} is {length} bytes, expected exactly {maximum}"
        );
    } else {
        ensure!(
            length <= maximum,
            "qualification artifact {name} is {length} bytes, exceeding its {maximum}-byte limit"
        );
    }
    Ok(())
}

pub(super) fn artifact_size_policy(kind: Kind, name: &str) -> Result<(u64, bool)> {
    Ok(match kind {
        Kind::Advisory => match name {
            ADVISORY_POLICY_PATH => (MAX_ADVISORY_POLICY_BYTES, false),
            ADVISORY_TRANSCRIPT => (MAX_ADVISORY_TRANSCRIPT_BYTES, false),
            ADVISORY_REFRESH_TRANSCRIPT => (MAX_ADVISORY_TRANSCRIPT_BYTES, false),
            _ => bail!("unregistered advisory artifact `{name}`"),
        },
        Kind::Mutation => (MAX_MUTATION_REPORT_BYTES, false),
        Kind::CargoMutantsGrade => match name {
            "request" | "doctest.transcript" => (MAX_EXTERNAL_RECEIPT_BYTES, false),
            "inventory.json" | "mutants.json" | "outcomes.json" => (MAX_MUTANTS_JSON_BYTES, false),
            "adjudications.json" => (MAX_MUTANTS_ADJUDICATION_BYTES, false),
            _ => bail!("unregistered cargo-mutants grade artifact `{name}`"),
        },
        Kind::CodexSecurityDeep => match name {
            "request" => (MAX_SECURITY_REQUEST_BYTES, false),
            "scan-manifest.json" => (MAX_SECURITY_MANIFEST_BYTES, false),
            "findings.json" => (MAX_SECURITY_FINDINGS_BYTES, false),
            "coverage.json" => (MAX_SECURITY_COVERAGE_BYTES, false),
            "report.md" => (MAX_SECURITY_REPORT_BYTES, false),
            "worker-transitions" => (MAX_SECURITY_TRANSITIONS_BYTES, false),
            _ => bail!("unregistered Codex Security artifact `{name}`"),
        },
        Kind::Model => (MAX_MODEL_TRANSCRIPT_BYTES, false),
        Kind::VmwareSelfTest | Kind::VmwareShipped => match name {
            VMWARE_IMAGE => (boot_image_bytes(), true),
            VMWARE_NAMESPACE => (namespace_bytes(), true),
            VMWARE_HANDOFF => (acpi_tables::DmaHandoff::LEN as u64, true),
            VMWARE_TRANSCRIPT => (MAX_VMWARE_TRANSCRIPT_BYTES, false),
            VMWARE_GENERATION_RECEIPT
            | VMWARE_VMDK
            | VMWARE_NAMESPACE_VMDK
            | VMWARE_VMX
            | VMWARE_OBSERVER => (MAX_VMWARE_TEXT_BYTES, false),
            _ => bail!("unregistered VMware artifact `{name}`"),
        },
    })
}

pub(crate) fn require_bundle_file_size(spec: &BundleFile, path: &Path) -> Result<()> {
    let (maximum, exact) = bundle_file_size_policy(spec)?;
    let length = File::open(path)
        .with_context(|| format!("opening qualification bundle file {}", path.display()))?
        .metadata()
        .with_context(|| format!("measuring qualification bundle file {}", path.display()))?
        .len();
    if exact {
        ensure!(
            length == maximum,
            "qualification bundle file {} is {length} bytes, expected exactly {maximum}",
            spec.relative
        );
    } else {
        ensure!(
            length <= maximum,
            "qualification bundle file {} is {length} bytes, exceeding its {maximum}-byte limit",
            spec.relative
        );
    }
    Ok(())
}

pub(crate) fn bundle_file_size_policy(spec: &BundleFile) -> Result<(u64, bool)> {
    ensure!(
        BUNDLE_FILES
            .iter()
            .any(|known| { known.prefix == spec.prefix && known.relative == spec.relative }),
        "unregistered qualification bundle file {}",
        spec.relative
    );
    if Path::new(spec.relative)
        .file_name()
        .and_then(|name| name.to_str())
        == Some(RECORD)
    {
        return Ok((MAX_RECORD_BYTES, false));
    }

    let kind = if spec.prefix.starts_with("qualification.advisory.") {
        Kind::Advisory
    } else if spec.prefix.starts_with("qualification.mutation.") {
        Kind::Mutation
    } else if spec
        .prefix
        .starts_with("qualification.cargo-mutants-grade.")
    {
        Kind::CargoMutantsGrade
    } else if spec
        .prefix
        .starts_with("qualification.codex-security-deep.")
    {
        Kind::CodexSecurityDeep
    } else if spec.prefix.starts_with("qualification.model.") {
        Kind::Model
    } else if spec.prefix.starts_with("qualification.vmware-self-test.") {
        Kind::VmwareSelfTest
    } else if spec.prefix.starts_with("qualification.vmware-shipped.") {
        Kind::VmwareShipped
    } else {
        bail!("unknown qualification bundle class `{}`", spec.prefix);
    };
    let name = bundle_tail(spec)?
        .strip_prefix(kind.slug())
        .with_context(|| format!("{} escaped its qualification kind", spec.relative))?
        .to_str()
        .context("qualification bundle file name is not UTF-8")?;
    artifact_size_policy(kind, name)
}
