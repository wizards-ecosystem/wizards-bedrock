//! Build orchestrator for The Wizard's Bedrock, in pure Rust with no external imaging tools.

// In-source because xtask takes no workspace lints, so unsafe code is still forbidden here.
#![forbid(unsafe_code)]

mod baseline;
mod bounded_process;
pub mod cargo;
pub mod cli;
mod debugger;
mod elf;
mod evidence;
mod files;
mod gate;
mod git;
pub mod image;
mod json;
mod model;
mod paths;
mod platform_acpi;
mod qemu;
mod qmp;
mod sha256;
mod supply_chain;
mod tasks;
#[cfg(test)]
mod test_support;
mod vendor;
mod vmware;
mod workspace;

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
            vmware::reset_vm_directory(&paths, kernel)?;
            let image = image::build(&paths, profile, kernel)?;
            vmware::generate(&paths, &image, kernel)
        }
        Task::QualifyVmware { kernel } => vmware::qualify(&paths, kernel),
        Task::Check { message_format } => tasks::check(&paths, message_format.as_deref()),
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
        Task::Ci => tasks::ci(&paths),
        Task::Evidence => evidence::create(&paths),
        Task::VerifyEvidence => evidence::verify(&paths),
        Task::Clean(scope) => tasks::clean(&paths, scope),
    }
}
