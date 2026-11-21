use core::time::Duration;
use std::time::Instant;

use anyhow::{Result, bail};

use super::serial::Selftest;
use super::{Control, Event, Installation, Machine, Processors, Remapping};
use crate::paths::RunDirectory;

const CEILING: Duration = Duration::from_secs(15 * 60);

// A kernel that neither passes nor fails, and stays silent for `quiet` or runs past the ceiling,
// has hung, and its emulator is killed.
pub(crate) fn run_selftest(
    run: &RunDirectory,
    quiet: Duration,
    processors: Processors,
) -> Result<()> {
    let mut command = Installation::locate()?.command(run, Remapping::Present, processors)?;
    command.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);
    println!("[xtask] booting the self-test kernel");
    let mut machine = Machine::launch(command, Control::Qmp)?;
    let ceiling = Instant::now() + CEILING;
    let mut silence = Instant::now() + quiet;
    let mut selftest = Selftest::new(processors.count());
    loop {
        match machine.next(silence.min(ceiling))? {
            Event::Line(line) => {
                selftest.observe(&line)?;
                silence = Instant::now() + quiet;
            }
            Event::Exited(status) => {
                selftest.conclude(status.code())?;
                println!("[xtask] self-test passed");
                return Ok(());
            }
            Event::TimedOut => {
                machine.report_hang();
                if Instant::now() >= ceiling {
                    bail!(
                        "self-test did not finish within the {} second ceiling",
                        CEILING.as_secs()
                    );
                }
                bail!("self-test wrote nothing for {} seconds", quiet.as_secs());
            }
        }
    }
}
