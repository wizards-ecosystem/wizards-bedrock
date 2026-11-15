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
