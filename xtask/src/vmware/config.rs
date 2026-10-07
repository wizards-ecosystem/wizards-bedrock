//! Source-bound VMX assignments and constrained Workstation runtime additions.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};

use crate::qemu::canonical_decimal;

/// Validate execution settings while allowing reviewed runtime identity fields.
pub(super) fn verify(generated: &[u8], runtime: &[u8]) -> Result<()> {
    let generated = assignments(generated)?;
    let runtime = assignments(runtime)?;
    for (key, value) in &generated {
        ensure!(
            runtime.get(key) == Some(value),
            "runtime VMX changed or omitted generated assignment `{key}`"
        );
    }
    for (key, value) in &runtime {
        ensure!(
            generated.contains_key(key) || runtime_value_is_valid(key, value),
            "runtime VMX has an unknown or invalid assignment `{key}`"
        );
    }
    Ok(())
}

fn assignments(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut assignments = BTreeMap::new();
    let mut seen_keys = BTreeSet::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let line = match line.strip_suffix(b"\n") {
            Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
            None => line,
        };
        ensure!(
            line.iter()
                .all(|byte| !byte.is_ascii_control() || *byte == b'\t'),
            "VMX contains an embedded control character"
        );
        let line = line.trim_ascii();
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        ensure!(line.is_ascii(), "VMX assignment is not ASCII");
        let line = std::str::from_utf8(line).context("VMX assignment is not UTF-8")?;
        let (key, value) = line
            .split_once('=')
            .context("VMX assignment has no equals sign")?;
        let key = key.trim().to_string();
        ensure!(
            !key.is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._:".contains(&byte)),
            "VMX assignment has an unsupported key"
        );
        let value = value
            .trim()
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .context("VMX assignment is not one quoted string")?;
        ensure!(
            value
                .bytes()
                .all(|byte| (b' '..=b'~').contains(&byte) && !b"\"|\\".contains(&byte)),
            "VMX assignment has an unsupported value or escape"
        );
        ensure!(
            seen_keys.insert(key.to_ascii_lowercase()),
            "VMX has duplicate case-insensitive assignments"
        );
        assignments.insert(key, value.to_string());
    }
    Ok(assignments)
}

fn runtime_value_is_valid(key: &str, value: &str) -> bool {
    match key {
        "uuid.bios" | "uuid.location" | "nvme0.subnqnUUID" | "nvme1.subnqnUUID" => {
            uuid_is_valid(value)
        }
        "numa.autosize.cookie" => canonical_decimal(value).is_some(),
        "monitor.phys_bits_used" => {
            canonical_decimal(value).is_some_and(|bits| (32..=52).contains(&bits))
        }
        "cleanShutdown" | "softPowerOff" => matches!(value, "TRUE" | "FALSE"),
        _ => false,
    }
}

fn uuid_is_valid(value: &str) -> bool {
    value.len() == 47
        && value.bytes().enumerate().all(|(index, byte)| {
            if index == 23 {
                byte == b'-'
            } else if index % 3 == 2 {
                byte == b' '
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::image::Kernel;

    const UUID: &str = "56 4d 50 0c 83 f7 81 b6-45 b7 5a 77 aa 9d 8b b4";

    fn generated() -> String {
        super::super::generation_vmx(Kernel::SelfTest, &format!("serial-{}.log", "1".repeat(64)))
            .unwrap()
    }

    #[test]
    fn workstation_runtime_identity_preserves_generated_settings() {
        let generated = generated();
        let additions = format!(
            "uuid.bios = \"{UUID}\"\nuuid.location = \"{UUID}\"\nnvme0.subnqnUUID = \"{UUID}\"\nnvme1.subnqnUUID = \"{UUID}\"\nnuma.autosize.cookie = \"20012\"\nmonitor.phys_bits_used = \"45\"\ncleanShutdown = \"FALSE\"\nsoftPowerOff = \"FALSE\"\n"
        );
        let runtime = format!("{generated}{additions}");
        verify(generated.as_bytes(), runtime.as_bytes()).unwrap();
        let mut rewritten = runtime.replace('\n', "\r\n").into_bytes();
        rewritten.extend_from_slice(b"# Workstation comment in Windows-1252: \x97\r\n");
        verify(generated.as_bytes(), &rewritten).unwrap();
        assert!(
            verify(
                generated.as_bytes(),
                runtime.replace("firmware =", "FIRMWARE =").as_bytes(),
            )
            .is_err()
        );
    }

    #[test]
    fn runtime_vmx_preserves_every_generated_assignment() {
        let generated = generated();
        for line in generated
            .lines()
            .filter(|line| !line.starts_with('#') && line.contains('='))
        {
            let missing = generated.replace(&format!("{line}\n"), "");
            assert!(
                verify(generated.as_bytes(), missing.as_bytes()).is_err(),
                "omitted {line}"
            );
            let (key, _) = line.split_once('=').unwrap();
            let changed = generated.replace(line, &format!("{key}= \"changed\""));
            assert!(
                verify(generated.as_bytes(), changed.as_bytes()).is_err(),
                "changed {line}"
            );
            let changed_case = generated.replace(
                line,
                &format!(
                    "{}{rest}",
                    key.to_ascii_uppercase(),
                    rest = &line[key.len()..]
                ),
            );
            if changed_case != generated {
                assert!(
                    verify(generated.as_bytes(), changed_case.as_bytes()).is_err(),
                    "changed key spelling {line}"
                );
            }
        }
    }

    #[test]
    fn runtime_vmx_refuses_unknown_and_duplicate_assignments() {
        let generated = generated();
        for addition in [
            "cpuid.7.edx = \"0\"\n",
            "sharedFolder0.present = \"TRUE\"\n",
        ] {
            let runtime = format!("{generated}{addition}");
            assert!(
                verify(generated.as_bytes(), runtime.as_bytes()).is_err(),
                "{addition}"
            );
        }
        for addition in [
            "firmware = \"efi\"\n".to_string(),
            "FIRMWARE = \"efi\"\n".to_string(),
            format!("uuid.bios = \"{UUID}\"\nuuid.BIOS = \"{UUID}\"\n"),
        ] {
            let runtime = format!("{generated}{addition}");
            assert!(assignments(runtime.as_bytes()).is_err(), "{addition}");
            assert!(verify(generated.as_bytes(), runtime.as_bytes()).is_err());
        }
    }

    #[test]
    fn runtime_identity_values_have_bounded_canonical_shapes() {
        let generated = generated();
        for (key, valid, invalid) in [
            (
                "uuid.bios",
                UUID,
                "gg 4d 50 0c 83 f7 81 b6-45 b7 5a 77 aa 9d 8b b4",
            ),
            (
                "uuid.location",
                UUID,
                "56 4d 50 0c 83 f7 81 b6 45 b7 5a 77 aa 9d 8b b4",
            ),
            ("nvme0.subnqnUUID", UUID, "not-a-uuid"),
            ("nvme1.subnqnUUID", UUID, "not-a-uuid"),
            ("numa.autosize.cookie", "20012", "020012"),
            ("monitor.phys_bits_used", "45", "53"),
            ("monitor.phys_bits_used", "32", "31"),
            ("cleanShutdown", "FALSE", "false"),
            ("softPowerOff", "TRUE", "1"),
        ] {
            let valid = format!("{generated}{key} = \"{valid}\"\n");
            let invalid = format!("{generated}{key} = \"{invalid}\"\n");
            verify(generated.as_bytes(), valid.as_bytes()).unwrap();
            assert!(
                verify(generated.as_bytes(), invalid.as_bytes()).is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn vmx_controls_and_alternate_assignment_forms_are_refused() {
        let generated = generated();
        for addition in [
            "# comment\rcpuid.7.edx = \"0\"\n",
            "# comment\0\n",
            "uuid.bios = \"|22\"\n",
            "uuid.bios = '\"value\"'\n",
            "uuid.bios = \"value\" # suffix\n",
            "uuid.bios = \"value\\\"suffix\"\n",
            "a-b = \"value\"\n",
            "= \"value\"\n",
            "not an assignment\n",
            "firmware = \"efi\"\r",
            "\u{a0}firmware = \"efi\"\n",
        ] {
            assert!(assignments(addition.as_bytes()).is_err(), "{addition:?}");
            let runtime = format!("{generated}{addition}");
            assert!(verify(generated.as_bytes(), runtime.as_bytes()).is_err());
        }
    }
}
