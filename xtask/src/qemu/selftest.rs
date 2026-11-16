use core::time::Duration;
use std::time::Instant;

use anyhow::{Result, bail};

use super::serial::Selftest;
use super::{Control, Event, Installation, Machine, Processors, Remapping};
use crate::paths::RunDirectory;

// A kernel that neither passes nor fails within `timeout` has hung, and its emulator is killed.
pub(crate) fn run_selftest(
    run: &RunDirectory,
    timeout: Duration,
    processors: Processors,
) -> Result<()> {
    let mut command = Installation::locate()?.command(run, Remapping::Present, processors)?;
    command.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);
    println!("[xtask] booting the self-test kernel");
    let mut machine = Machine::launch(command, Control::Qmp)?;
    let deadline = Instant::now() + timeout;
    let mut selftest = Selftest::new(processors.count());
    loop {
        match machine.next(deadline)? {
            Event::Line(line) => selftest.observe(&line)?,
            Event::Exited(status) => {
                selftest.conclude(status.code())?;
                println!("[xtask] self-test passed");
                return Ok(());
            }
            Event::TimedOut => {
                machine.report_hang();
                bail!(
                    "self-test did not finish within {} seconds",
                    timeout.as_secs()
                );
            }
        }
    }
}
