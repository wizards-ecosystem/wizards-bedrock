//! Build orchestrator for The Wizard's OS.
//!
//! Everything happens in-process in pure Rust — no xorriso, mtools, or other
//! external imaging tools. The bootable artifact is a raw disk image with an
//! MBR partition table holding a single FAT32 EFI system partition.
//!
//! Run `cargo xtask help` for the task list.

// In-source because xtask takes no workspace lints (D-22); D-67's rule that
// unsafe is permitted nowhere in silence still applies here.
#![forbid(unsafe_code)]

pub mod api;
pub mod baseline;
pub mod boundary;
pub mod cargo;
pub mod cli;
#[cfg(test)]
mod contracts;
mod debugger;
pub mod decisions;
pub mod elf;
mod evidence;
mod gate;
#[cfg(test)]
mod handoff;
pub mod image;
pub mod machine;
pub mod model;
pub mod mutate;
pub mod mutation_workspace;
pub mod paths;
pub mod prose;
#[cfg(test)]
mod publication;
pub mod qemu;
pub mod qmp;
mod qualification;
pub mod sha256;
pub mod supply_chain;
pub mod tasks;
pub mod unsafety;
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
            println!("{}", cli::help());
            Ok(())
        }
        Task::Build(profile) => cargo::build_kernel(&paths, profile, &[]).map(drop),
        Task::Image(profile) => image::build(&paths, profile, image::Kernel::Shipped).map(drop),
        Task::Run { profile, qemu_args } => {
            let image = image::build_for_run(&paths, profile, image::Kernel::Shipped)?;
            qemu::run(&paths, image.path(), &qemu_args)
        }
        Task::Vmware { profile, kernel } => {
            // Self-test remains the default. This built the shipped kernel
            // until D-135, so D-18's phase gate ran none of its checks. The
            // explicit shipped selector exists for the separate PW-004 round.
            let image = image::build(&paths, profile, kernel)?;
            vmware::generate(&paths, &image, kernel)
        }
        Task::Check { message_format } => tasks::check(&paths, message_format.as_deref()),
        Task::ApiAudit => api::check(&paths),
        Task::Clippy => tasks::clippy(&paths),
        Task::Doc => tasks::doc(&paths),
        Task::Fmt(mode) => tasks::fmt(&paths, mode),
        Task::Test => tasks::test(&paths),
        Task::TestBoot(profile) => tasks::test_boot(&paths, profile),
        Task::BootCheck(profile) => tasks::boot_check(&paths, profile),
        Task::DegradedCheck(profile) => tasks::degraded_check(&paths, profile),
        Task::VerifyVendor => vendor::verify(&paths),
        Task::SupplyChain => supply_chain::verify(&paths),
        Task::UpdateSbom => supply_chain::update_spdx(&paths),
        Task::ModelCheck => model::check(&paths),
        Task::Baseline => baseline::check(&paths),
        Task::Debug(mode) => debugger::run(&paths, mode),
        Task::Mutate => mutate::run(&paths),
        Task::Ci => tasks::ci(&paths),
        Task::Evidence => evidence::create(&paths),
        Task::VerifyEvidence => evidence::verify(&paths),
        Task::Clean(scope) => tasks::clean(&paths, scope),
    }
}
