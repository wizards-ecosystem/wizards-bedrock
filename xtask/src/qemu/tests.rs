mod kernel_source;
mod runner;
mod serial;

use std::io::{Seek, SeekFrom};

use super::boot_check::*;
use super::debug::*;
use crate::contracts::abi::lexers::{compact_cpp, rust_function_body, rust_without_comments};

use super::*;
use crate::test_support::{create_directory_symlink, create_file_symlink};

#[derive(Debug)]
struct QemuFixture {
    root: PathBuf,
    template: PathBuf,
}

impl QemuFixture {
    fn create() -> Self {
        let root = crate::test_support::reserve_temp_directory("wizard qemu,fixture");
        let template = root.join("firmware template,pristine.fd");
        std::fs::write(&template, b"pristine firmware variables")
            .expect("QEMU variable-store template");
        Self { root, template }
    }
}

impl Drop for QemuFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn publish_fixture(parent: &Path, bytes: &'static [u8]) -> Result<PathBuf> {
    let backing = parent.join("nvme.img");
    publish_namespace(parent, &backing, |file| {
        file.write_all(bytes).context("writing fixture namespace")
    })?;
    Ok(backing)
}

#[derive(Debug)]
struct FakeQemuChild {
    exited: bool,
    killed: bool,
    reject_kill: bool,
    exit_on_rejected_kill: bool,
    polls_after_kill: usize,
    exit_after_kill_polls: Option<usize>,
}

impl FakeQemuChild {
    fn exited(&mut self) -> std::io::Result<bool> {
        if self.exited || (self.reject_kill && self.exit_on_rejected_kill && self.killed) {
            return Ok(true);
        }
        if self.killed && !self.reject_kill {
            self.polls_after_kill += 1;
            return Ok(self
                .exit_after_kill_polls
                .is_some_and(|limit| self.polls_after_kill >= limit));
        }
        Ok(false)
    }

    fn terminate(&mut self) -> std::io::Result<()> {
        self.killed = true;
        if self.reject_kill {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fixture refused termination",
            ))
        } else {
            Ok(())
        }
    }
}

fn fake_qemu() -> FakeQemuChild {
    FakeQemuChild {
        exited: false,
        killed: false,
        reject_kill: false,
        exit_on_rejected_kill: false,
        polls_after_kill: 0,
        exit_after_kill_polls: Some(1),
    }
}
