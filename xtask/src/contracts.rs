//! Compile-time contracts which have no positive stable-Rust assertion.

use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::paths::{KERNEL_TARGET, Paths};

static NEXT_PROBE: AtomicUsize = AtomicUsize::new(0);

struct Probe {
    root: std::path::PathBuf,
}

impl Probe {
    fn create(paths: &Paths, guard: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "wizard-guard-contract-{}-{}",
            std::process::id(),
            NEXT_PROBE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("src")).expect("guard contract source directory");

        let wost = paths
            .root()
            .join("wost")
            .to_string_lossy()
            .replace('\\', "/");
        let manifest = format!(
            "[package]\nname = \"guard-contract\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             \n[workspace]\n\n[dependencies]\nwost = {{ path = {wost:?} }}\n"
        );
        std::fs::write(root.join("Cargo.toml"), manifest).expect("guard contract manifest");
        std::fs::write(
            root.join("src/lib.rs"),
            format!(
                "#![no_std]\nfn requires_send<T: Send>() {{}}\n\
                 fn probe<'a>() {{ requires_send::<wost::sync::{guard}<'a, u8>>(); }}\n"
            ),
        )
        .expect("guard contract source");
        Self { root }
    }

    fn compile(&self, paths: &Paths) -> Output {
        Command::new(env!("CARGO"))
            .current_dir(&self.root)
            .env(
                "CARGO_TARGET_DIR",
                paths.target_dir().join("guard-contracts"),
            )
            .args([
                "check",
                "--offline",
                "--target",
                KERNEL_TARGET,
                "--message-format=short",
            ])
            .output()
            .expect("compile guard contract")
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn refuses_send(paths: &Paths, guard: &str) -> Result<(), String> {
    let probe = Probe::create(paths, guard);
    let output = probe.compile(paths);
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let refused = !output.status.success()
        && diagnostics.contains("error[E0277]")
        && diagnostics.contains("cannot be sent between threads safely")
        && diagnostics.contains("lib.rs");
    refused
        .then_some(())
        .ok_or_else(|| diagnostics.into_owned())
}

#[test]
fn guard_types_do_not_cross_cpus() {
    let paths = Paths::discover().expect("workspace paths");
    refuses_send(&paths, "SpinLockGuard").expect("SpinLockGuard was Send");
    refuses_send(&paths, "ThreadLockGuard").expect("ThreadLockGuard was Send");
}
