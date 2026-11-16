use core::time::Duration;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use boot_record::{Exit, Fact, Processors as Online, Protections, Ran, Record};
use native_abi::witness;

use super::serial::{FULL_PROTECTIONS, Records, read_record, require_no_panic};
use super::{Control, Event, Installation, Machine, Processors, Remapping};
use crate::paths::RunDirectory;

const BOOT_TIMEOUT: Duration = Duration::from_secs(180);
// Running time after the kernel settles, so one dying on its first interrupt shows.
const SETTLE: Duration = Duration::from_secs(5);
// A shipped kernel must accept this key without unmasking legacy IRQ1.
const CHALLENGE_KEY: &str = "a";
const CHALLENGE_WINDOW: Duration = Duration::from_millis(250);
const KEYBOARD_IRQ: u8 = 1;
// No x2APIC and no optional SMEP, SMAP or UMIP; the last `-cpu` wins.
const DEGRADED_CPU: &str = "qemu64,-x2apic,-smep,-smap,-umip,hypervisor=on,tcg-cpuid=on";

const SHIPPED_REQUIRED: &[Record] = &[
    Record::Fact(Fact::DmaFenceStrict),
    Record::Fact(Fact::IsolationShared),
    FULL_PROTECTIONS,
    Record::Fact(Fact::NvmeCandidateRetired),
    Record::Fact(Fact::IommuTranslating),
    Record::Fact(Fact::NvmeUp),
    Record::Fact(Fact::Fat32Mounted),
    Record::Fact(Fact::KeyboardLinesMasked),
    Record::Fact(Fact::KeyboardPolled),
    // The boot witness is the shipped kernel's only program, and status 0 is its agreement.
    Record::Exit(Exit {
        status: witness::AGREED,
    }),
    Record::Processors(Online {
        online: Processors::Two.count(),
    }),
    Record::Ran(Ran {
        processors: Processors::Two.count(),
    }),
];

const NO_X2APIC_REQUIRED: &[Record] = &[
    Record::Fact(Fact::DmaFenceStrict),
    Record::Fact(Fact::IsolationShared),
    Record::Fact(Fact::X2apicAbsent),
    Record::Fact(Fact::TmpfsListed),
    Record::Fact(Fact::DevfsResolved),
    Record::Protections(Protections {
        write_protect: true,
        smep: false,
        smap: false,
        umip: false,
        nx: true,
    }),
    Record::Fact(Fact::ExtendedStateAbsent),
    Record::Fact(Fact::RingThreeClosedToInterrupts),
];

// The first candidate's clean retirement proves the refusal path returns its lease exactly.
const UNISOLATED_REQUIRED: &[Record] = &[
    Record::Fact(Fact::DmaFenceStrict),
    Record::Fact(Fact::IsolationShared),
    Record::Fact(Fact::StorageUnisolated),
    Record::Fact(Fact::NvmeCandidateRetired),
    Record::Fact(Fact::NvmeNone),
];

const SETTLED: Record = Record::Fact(Fact::Settled);

const KEY_CONSUMED: Record = Record::Fact(Fact::KeyConsumed);

#[derive(Debug)]
struct Boot {
    machine: Machine,
    records: Records,
    kernel: &'static str,
    settled: bool,
    key_injected: bool,
    key_consumed: bool,
}

impl Boot {
    fn launch(
        command: Command,
        control: Control,
        kernel: &'static str,
        required: &'static [Record],
    ) -> Result<Self> {
        Ok(Self {
            machine: Machine::launch(command, control)?,
            records: Records::new(required),
            kernel,
            settled: false,
            key_injected: false,
            key_consumed: false,
        })
    }

    fn observe(&mut self, line: &str) -> Result<()> {
        require_no_panic(line, self.kernel)?;
        let Some(record) = read_record(line)? else {
            return Ok(());
        };
        self.records.observe(record);
        self.settled |= record == SETTLED;
        self.key_consumed |= self.key_injected && record == KEY_CONSUMED;
        Ok(())
    }

    fn until_settled(&mut self) -> Result<()> {
        let deadline = Instant::now() + BOOT_TIMEOUT;
        while !self.settled {
            match self.machine.next(deadline)? {
                Event::Line(line) => self.observe(&line)?,
                Event::Exited(status) => bail!("the {} kernel exited ({status})", self.kernel),
                Event::TimedOut => bail!("the {} kernel never wrote `{SETTLED}`", self.kernel),
            }
        }
        Ok(())
    }

    fn watch(&mut self, window: Duration) -> Result<()> {
        let deadline = Instant::now() + window;
        loop {
            match self.machine.next(deadline)? {
                Event::Line(line) => self.observe(&line)?,
                Event::Exited(status) => bail!("the {} kernel exited ({status})", self.kernel),
                Event::TimedOut => return Ok(()),
            }
        }
    }

    fn drain(&mut self) -> Result<()> {
        while let Ok(line) = self.machine.lines.try_recv() {
            self.observe(&line?)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        let _ = self.machine.child.kill();
        let deadline = Instant::now() + SETTLE;
        while let Event::Line(line) = self.machine.next(deadline)? {
            self.observe(&line)?;
        }
        self.records.require_all(self.kernel)
    }
}

pub(crate) fn boot_check(run: &RunDirectory) -> Result<()> {
    let command = Installation::locate()?.command(run, Remapping::Present, Processors::Two)?;
    println!("[xtask] booting the shipped kernel");
    let mut boot = Boot::launch(command, Control::Qmp, "shipped", SHIPPED_REQUIRED)?;
    boot.until_settled()?;
    let before = keyboard_irq_count(&mut boot.machine)?;
    boot.drain()?;
    boot.machine
        .qmp()?
        .inject_key(CHALLENGE_KEY)
        .context("injecting the shipped-kernel liveness key")?;
    boot.key_injected = true;
    boot.watch(CHALLENGE_WINDOW)?;
    let after = keyboard_irq_count(&mut boot.machine)?;
    ensure!(
        after == before,
        "the shipped kernel unmasked legacy keyboard IRQ1 ({before} -> {after})"
    );
    boot.watch(SETTLE)?;
    let consumed = boot.key_consumed;
    boot.finish()?;
    ensure!(
        consumed,
        "the shipped kernel never consumed the injected key"
    );
    println!("[xtask] shipped kernel falls back, boots, responds, and settles");
    Ok(())
}

pub(crate) fn degraded_check(run: &RunDirectory) -> Result<()> {
    let mut command = Installation::locate()?.command(run, Remapping::Present, Processors::One)?;
    command.args(["-cpu", DEGRADED_CPU]);
    println!("[xtask] booting the shipped kernel with no x2apic");
    settle(Boot::launch(
        command,
        Control::None,
        "no-x2apic",
        NO_X2APIC_REQUIRED,
    )?)?;
    println!("[xtask] optional CPU protection fallback remains diagnostic-only");

    let command = Installation::locate()?.command(run, Remapping::Absent, Processors::One)?;
    println!("[xtask] booting the shipped kernel with no DMA-remapping unit");
    settle(Boot::launch(
        command,
        Control::None,
        "unisolated",
        UNISOLATED_REQUIRED,
    )?)?;
    println!("[xtask] unisolated kernel refuses storage and settles");
    Ok(())
}

fn settle(mut boot: Boot) -> Result<()> {
    boot.until_settled()?;
    boot.watch(SETTLE)?;
    boot.finish()
}

// `info irq` lists only interrupts that fired, so a missing row is a zero count.
fn keyboard_irq_count(machine: &mut Machine) -> Result<u64> {
    let report = machine
        .qmp()?
        .human("info irq")
        .context("querying keyboard IRQ progress")?;
    ensure!(
        report.contains("IRQ statistics"),
        "QEMU monitor returned no IRQ statistics"
    );
    let mut total = 0u64;
    for line in report.lines() {
        let Some((label, count)) = line.split_once(':') else {
            continue;
        };
        let label = label.trim();
        let label = label.strip_prefix("IRQ").unwrap_or(label).trim();
        if label.parse::<u8>().ok() != Some(KEYBOARD_IRQ) {
            continue;
        }
        let count: u64 = count
            .split_whitespace()
            .next()
            .and_then(|count| count.parse().ok())
            .context("QEMU monitor returned a malformed IRQ count")?;
        total = total
            .checked_add(count)
            .context("the IRQ count overflowed")?;
    }
    Ok(total)
}
