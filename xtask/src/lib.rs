//! Build orchestrator for The Wizard's OS.
//!
//! Everything happens in-process in pure Rust — no xorriso, mtools, or other
//! external imaging tools. The bootable artifact is a raw disk image with an
//! MBR partition table holding a single FAT32 EFI system partition.
//!
//! Run `cargo xtask help` for the task list.

pub mod cargo;
pub mod cli;
pub mod elf;
pub mod image;
pub mod paths;
pub mod qemu;
pub mod qmp;
pub mod sha256;
pub mod tasks;
pub mod vendor;
pub mod vmware;

use anyhow::Result;

use crate::cli::Task;
use crate::paths::Paths;

/// Carry out `task`.
pub fn run(task: Task) -> Result<()> {
    let paths = Paths::discover()?;
    match task {
        Task::Help => {
            println!("{}", cli::HELP);
            Ok(())
        }
        Task::Build(profile) => cargo::build_kernel(&paths, profile, &[]).map(drop),
        Task::Image(profile) => image::build(&paths, profile, &[]).map(drop),
        Task::Run { profile, qemu_args } => {
            let image = image::build(&paths, profile, &[])?;
            qemu::run(&paths, &image, &qemu_args)
        }
        Task::Vmware(profile) => {
            let image = image::build(&paths, profile, &[])?;
            vmware::generate(&paths, &image)
        }
        Task::Check { message_format } => tasks::check(&paths, message_format.as_deref()),
        Task::Clippy => tasks::clippy(&paths),
        Task::Doc => tasks::doc(&paths),
        Task::Fmt(mode) => tasks::fmt(&paths, mode),
        Task::Test => tasks::test(&paths),
        Task::TestBoot(profile) => tasks::test_boot(&paths, profile),
        Task::VerifyVendor => vendor::verify(&paths),
        Task::Ci => tasks::ci(&paths),
        Task::Clean(scope) => tasks::clean(&paths, scope),
    }
}
