// xtask takes no workspace lints, so unsafe code is forbidden here instead.
#![forbid(unsafe_code)]

mod cargo;
pub mod cli;
mod debugger;
mod elf;
mod image;
mod json;
mod model;
mod paths;
mod platform_acpi;
mod qemu;
mod qmp;
mod sha256;
mod supply_chain;
mod tasks;
mod vendor;
mod vmware;

use anyhow::Result;

use crate::cli::Task;
use crate::image::Kernel;
use crate::paths::{Paths, RunDirectory};

pub fn run(task: Task) -> Result<()> {
    let paths = Paths::discover()?;
    match task {
        Task::Help => {
            println!("{}", cli::HELP);
            Ok(())
        }
        Task::Build(profile) => cargo::build_kernel(&paths, profile, &[]).map(drop),
        Task::Image(profile) => image::build(&paths, profile, Kernel::Shipped, &paths.image()),
        Task::Run { profile, qemu_args } => {
            let run = RunDirectory::create(&paths)?;
            image::build(&paths, profile, Kernel::Shipped, &run.image())?;
            qemu::run(&run, &qemu_args)
        }
        Task::Vmware { kernel } => vmware::generate(&paths, kernel),
        Task::QualifyVmware { kernel } => vmware::qualify(&paths, kernel),
        Task::Check { message_format } => tasks::check(&paths, message_format.as_deref()),
        Task::Clippy => tasks::clippy(&paths),
        Task::Fmt(mode) => tasks::fmt(&paths, mode),
        Task::Test => tasks::test(&paths),
        Task::TestBoot(profile) => tasks::test_boot(&paths, profile),
        Task::BootCheck(profile) => tasks::boot_check(&paths, profile),
        Task::DegradedCheck(profile) => tasks::degraded_check(&paths, profile),
        Task::VerifyVendor => vendor::verify(&paths),
        Task::SupplyChain => supply_chain::verify(&paths),
        Task::ModelCheck => model::verify(&paths),
        Task::Debug => debugger::run(&paths),
        Task::Ci => tasks::ci(&paths),
        Task::Clean(scope) => tasks::clean(&paths, scope),
    }
}
