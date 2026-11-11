//! Source-bound VMX assignments and constrained Workstation runtime additions.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};

use crate::qemu::canonical_decimal;

pub(super) fn verify(generated: &[u8], runtime: &[u8]) -> Result<()> {
    let generated = assignments(generated)?;
    let runtime = assignments(runtime)?;
    let vnc_keys = [
        "RemoteDisplay.vnc.enabled",
        "RemoteDisplay.vnc.ip",
        "RemoteDisplay.vnc.port",
    ];
    ensure!(
        vnc_keys.iter().all(|key| !runtime.contains_key(*key))
            || vnc_keys.iter().all(|key| runtime.contains_key(*key)),
        "runtime VNC console settings must be absent or a complete loopback group"
    );
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
        "RemoteDisplay.vnc.enabled" => value == "TRUE",
        "RemoteDisplay.vnc.ip" => value == "127.0.0.1",
        "RemoteDisplay.vnc.port" => value == "5997",
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
mod tests;
