use std::time::{Duration, Instant};

use anyhow::{Result, bail, ensure};

use super::serial::{ROUNDS, Selftest};
use super::{Control, Event, Installation, Machine, Remapping};
use crate::paths::RunDirectory;

// A kernel that neither passes nor fails within `timeout` has hung, and its emulator is killed.
pub(crate) fn run_selftest(run: &RunDirectory, timeout: Duration) -> Result<()> {
    let mut command = Installation::locate()?.command(run, Remapping::Present)?;
    command.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);
    println!("[xtask] booting the self-test kernel");
    let mut machine = Machine::launch(command, Control::Qmp)?;
    let deadline = Instant::now() + timeout;
    let mut selftest = Selftest::default();
    let mut round = 0;
    loop {
        match machine.next(deadline)? {
            Event::Line(line) => {
                selftest.observe(&line)?;
                let Some(index) = ROUNDS.iter().position(|(marker, _)| line == *marker) else {
                    continue;
                };
                ensure!(
                    index == round,
                    "self-test keystroke round {} opened while waiting for round {}",
                    index + 1,
                    round + 1
                );
                round += 1;
                inject(&mut machine, ROUNDS[index].1);
            }
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

// A failed injection is printed and left to the kernel's own keystroke check, which fails.
fn inject(machine: &mut Machine, keys: &[&str]) {
    let injected = machine
        .qmp()
        .and_then(|qmp| keys.iter().try_for_each(|key| qmp.inject_key(key)));
    match injected {
        Ok(()) => println!("[xtask] qmp: {} keystroke(s) injected", keys.len()),
        Err(error) => println!("[xtask] qmp: {error:#}; the keystroke check will report it"),
    }
}
