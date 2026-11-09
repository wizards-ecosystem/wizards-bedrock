use super::*;

#[derive(Clone, Copy)]
struct WorkbenchCommandPolicy {
    required: &'static [&'static str],
    optional: &'static [&'static str],
    repeatable: &'static [&'static str],
}

fn workbench_command_policy(command: &str) -> Result<WorkbenchCommandPolicy> {
    let policy = match command {
        "resolve-scan-root" => WorkbenchCommandPolicy {
            required: &["--scan-root"],
            optional: &[],
            repeatable: &[],
        },
        "begin-deep-scan" => WorkbenchCommandPolicy {
            required: &[
                "--thread-id",
                "--target-path",
                "--scope",
                "--scan-root",
                "--available-parallelism",
                "--workflow-version",
                "--model",
                "--reasoning-effort",
            ],
            optional: &[],
            repeatable: &[],
        },
        "get-deep-scan" => WorkbenchCommandPolicy {
            required: &["--scan-id", "--thread-id"],
            optional: &[],
            repeatable: &[],
        },
        "claim-deep-scan-coordinator" => WorkbenchCommandPolicy {
            required: &["--scan-id", "--thread-id"],
            optional: &[],
            repeatable: &[],
        },
        "upsert-deep-scan-worker" => WorkbenchCommandPolicy {
            required: &[
                "--scan-id",
                "--worker-id",
                "--kind",
                "--status",
                "--prompt-path",
                "--artifact-dir",
                "--attempt",
                "--coordinator-generation",
            ],
            optional: &[
                "--result-manifest-path",
                "--sdk-thread-id",
                "--error-message",
                "--replaceable-failure-kind",
            ],
            repeatable: &[],
        },
        "claim-deep-scan-dedup" => WorkbenchCommandPolicy {
            required: &[
                "--scan-id",
                "--worker-id",
                "--prompt-path",
                "--artifact-dir",
                "--coordinator-generation",
                "--input-worker-id",
            ],
            optional: &[],
            repeatable: &["--input-worker-id"],
        },
        "commit-deep-scan-dedup" => WorkbenchCommandPolicy {
            required: &[
                "--scan-id",
                "--worker-id",
                "--coordinator-generation",
                "--result-manifest-path",
                "--new-findings-count",
            ],
            optional: &["--candidate-ledger-path"],
            repeatable: &[],
        },
        "finish-deep-scan" => WorkbenchCommandPolicy {
            required: &[
                "--scan-id",
                "--coordinator-generation",
                "--terminal-reason",
                "--manifest-path",
            ],
            optional: &["--staged-manifest-path", "--omitted-worker-id"],
            repeatable: &["--omitted-worker-id"],
        },
        "fail-deep-scan" => WorkbenchCommandPolicy {
            required: &["--scan-id", "--message"],
            optional: &[
                "--coordinator-generation",
                "--manifest-path",
                "--staged-manifest-path",
                "--deep-status",
            ],
            repeatable: &[],
        },
        "record-deep-scan-publication-failure" => WorkbenchCommandPolicy {
            required: &["--scan-id", "--coordinator-generation", "--message"],
            optional: &[],
            repeatable: &[],
        },
        "update-progress" => WorkbenchCommandPolicy {
            required: &["--scan-id", "--coordinator-generation"],
            optional: &[
                "--phase",
                "--deep-review-pass",
                "--review-items-total",
                "--review-items-completed",
            ],
            repeatable: &[],
        },
        "get-scan" => WorkbenchCommandPolicy {
            required: &["--scan-id"],
            optional: &[],
            repeatable: &[],
        },
        "write-scan-draft" => WorkbenchCommandPolicy {
            required: &["--scan-id", "--draft-path", "--checkpoint-path"],
            optional: &["--expected-draft-digest"],
            repeatable: &[],
        },
        "preserve-scan-results" => WorkbenchCommandPolicy {
            required: &["--scan-id", "--thread-id", "--coordinator-generation"],
            optional: &[],
            repeatable: &[],
        },
        "complete-scan" => WorkbenchCommandPolicy {
            required: &["--scan-id"],
            optional: &[],
            repeatable: &[],
        },
        _ => bail!("private Python workbench command is outside the owned Deep workflow"),
    };
    Ok(policy)
}

pub(crate) fn validate_workbench_arguments(
    arguments: &[OsString],
    repository: &Path,
    scan_root: &Path,
    state_root: &Path,
) -> Result<()> {
    let command = arguments
        .first()
        .context("private Python workbench invocation has no command")?
        .to_str()
        .context("private Python workbench command is not UTF-8")?;
    let policy = workbench_command_policy(command)?;
    let allowed: BTreeSet<&str> = policy
        .required
        .iter()
        .chain(policy.optional)
        .chain(policy.repeatable)
        .copied()
        .collect();
    let repeatable: BTreeSet<&str> = policy.repeatable.iter().copied().collect();
    let mut parsed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut index = 1usize;
    while index < arguments.len() {
        let option = arguments[index]
            .to_str()
            .context("private Python workbench option is not UTF-8")?;
        ensure!(
            allowed.contains(option),
            "private Python workbench option differs for {command}"
        );
        let value = arguments
            .get(index + 1)
            .context("private Python workbench option has no value")?;
        require_nonempty_argument(value, "private Python workbench option value")?;
        let value = value
            .to_str()
            .context("private Python workbench option value is not UTF-8")?;
        ensure!(
            value.len() <= MAX_MCP_LINE_BYTES,
            "private Python workbench option value is too large"
        );
        let values = parsed.entry(option.to_string()).or_default();
        ensure!(
            values.is_empty() || repeatable.contains(option),
            "private Python workbench option is duplicated"
        );
        values.push(value.to_string());
        index += 2;
    }
    for required in policy.required {
        ensure!(
            parsed.contains_key(*required),
            "private Python workbench command lacks {required}"
        );
    }
    ensure!(
        state_root.parent() == scan_root.parent(),
        "private Python workbench roots do not share owned scratch"
    );
    for (option, values) in &parsed {
        for value in values {
            match option.as_str() {
                "--target-path" => ensure!(
                    Path::new(value) == repository,
                    "private Python workbench target differs"
                ),
                "--scope" => ensure!(value == ".", "private Python workbench scope differs"),
                "--scan-root" => ensure!(
                    Path::new(value) == scan_root,
                    "private Python workbench scan root differs"
                ),
                "--model" => ensure!(value == MODEL, "private Python workbench model differs"),
                "--reasoning-effort" => ensure!(
                    value == REASONING,
                    "private Python workbench reasoning differs"
                ),
                "--workflow-version" => ensure!(
                    value == DEEP_WORKFLOW_VERSION,
                    "private Python workbench workflow differs"
                ),
                "--kind" => ensure!(
                    matches!(value.as_str(), "discovery" | "dedup"),
                    "private Python workbench worker kind differs"
                ),
                "--deep-status" => ensure!(
                    value == "interrupted",
                    "private Python workbench terminal status differs"
                ),
                "--available-parallelism"
                | "--attempt"
                | "--coordinator-generation"
                | "--new-findings-count"
                | "--deep-review-pass"
                | "--review-items-total"
                | "--review-items-completed" => {
                    let number = value
                        .parse::<u64>()
                        .context("private Python workbench number is malformed")?;
                    ensure!(
                        value == &number.to_string(),
                        "private Python workbench number is not canonical"
                    );
                    if option == "--available-parallelism" || option == "--coordinator-generation" {
                        ensure!(
                            number > 0,
                            "private Python workbench number must be positive"
                        );
                    }
                }
                "--prompt-path"
                | "--artifact-dir"
                | "--result-manifest-path"
                | "--candidate-ledger-path"
                | "--manifest-path"
                | "--staged-manifest-path"
                | "--draft-path"
                | "--checkpoint-path" => {
                    require_owned_path(
                        Path::new(value),
                        scan_root,
                        "private Python workbench path",
                    )?;
                }
                "--scan-id" | "--worker-id" | "--input-worker-id" | "--omitted-worker-id" => {
                    require_safe_identity(value, "private Python workbench identity")?
                }
                "--expected-draft-digest" => {
                    let digest = value.strip_prefix("sha256:").unwrap_or(value);
                    require_sha256(digest, "private Python workbench draft digest")?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

pub(crate) fn require_safe_identity(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 256
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            }),
        "{label} is malformed"
    );
    Ok(())
}
