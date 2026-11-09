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
fn loopback_vnc_additions_are_complete_and_exact() {
    let generated = generated();
    let vnc = [
        ("RemoteDisplay.vnc.enabled", "TRUE"),
        ("RemoteDisplay.vnc.ip", "127.0.0.1"),
        ("RemoteDisplay.vnc.port", "5997"),
    ];
    let additions = vnc
        .iter()
        .map(|(key, value)| format!("{key} = \"{value}\"\n"))
        .collect::<String>();
    let runtime = format!("{generated}{additions}");
    verify(generated.as_bytes(), runtime.as_bytes()).unwrap();
    let with_identity = format!("{runtime}uuid.bios = \"{UUID}\"\n").replace('\n', "\r\n");
    verify(generated.as_bytes(), with_identity.as_bytes()).unwrap();
    for subset in 0..(1 << vnc.len()) {
        let mut partial = generated.clone();
        for (index, (key, value)) in vnc.iter().enumerate() {
            if subset & (1 << index) != 0 {
                partial.push_str(&format!("{key} = \"{value}\"\n"));
            }
        }
        assert_eq!(
            verify(generated.as_bytes(), partial.as_bytes()).is_ok(),
            matches!(subset, 0 | 7),
            "VNC subset {subset}"
        );
    }
    for (key, valid, invalid) in [
        ("RemoteDisplay.vnc.enabled", "TRUE", "FALSE"),
        ("RemoteDisplay.vnc.enabled", "TRUE", "true"),
        ("RemoteDisplay.vnc.enabled", "TRUE", "1"),
        ("RemoteDisplay.vnc.ip", "127.0.0.1", "0.0.0.0"),
        ("RemoteDisplay.vnc.ip", "127.0.0.1", "127.0.0.2"),
        ("RemoteDisplay.vnc.ip", "127.0.0.1", "localhost"),
        ("RemoteDisplay.vnc.ip", "127.0.0.1", "::1"),
        ("RemoteDisplay.vnc.ip", "127.0.0.1", "192.0.2.1"),
        ("RemoteDisplay.vnc.ip", "127.0.0.1", ""),
        ("RemoteDisplay.vnc.port", "5997", "5996"),
        ("RemoteDisplay.vnc.port", "5997", "5998"),
        ("RemoteDisplay.vnc.port", "5997", "05997"),
        ("RemoteDisplay.vnc.port", "5997", "+5997"),
        ("RemoteDisplay.vnc.port", "5997", "5997.0"),
    ] {
        let changed = runtime.replace(
            &format!("{key} = \"{valid}\""),
            &format!("{key} = \"{invalid}\""),
        );
        assert!(
            verify(generated.as_bytes(), changed.as_bytes()).is_err(),
            "{key} = {invalid:?}"
        );
    }
    for addition in [
        "RemoteDisplay.vnc.password = \"password\"\n",
        "RemoteDisplay.vnc.keyMap = \"us\"\n",
        "RemoteDisplay.vnc.foo = \"TRUE\"\n",
        "RemoteDisplay.foo = \"TRUE\"\n",
        "cpuid.7.edx = \"0\"\n",
    ] {
        let changed = format!("{runtime}{addition}");
        assert!(
            verify(generated.as_bytes(), changed.as_bytes()).is_err(),
            "{addition}"
        );
    }
    for (key, value) in vnc {
        for duplicate in [key.to_string(), key.to_ascii_uppercase()] {
            let changed = format!("{runtime}{duplicate} = \"{value}\"\n");
            assert!(verify(generated.as_bytes(), changed.as_bytes()).is_err());
        }
        let changed = runtime.replace(key, &key.to_ascii_lowercase());
        assert!(verify(generated.as_bytes(), changed.as_bytes()).is_err());
    }
    let changed = runtime.replace("firmware = \"efi\"", "firmware = \"bios\"");
    assert!(verify(generated.as_bytes(), changed.as_bytes()).is_err());
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
