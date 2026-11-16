use core::time::Duration;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};

use super::serial::{
    FULL_PROTECTIONS, KEY_CONSUMED, NVME_FALLBACK, Records, SETTLED, SHIPPED_ISOLATION,
    STRICT_DMA_HANDOFF, USER_CALL, require_no_panic,
};
use super::{Control, Event, Installation, Machine, Remapping};
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

const SHIPPED_REQUIRED: &[&str] = &[
    STRICT_DMA_HANDOFF,
    SHIPPED_ISOLATION,
    FULL_PROTECTIONS,
    NVME_FALLBACK,
    "[info ] iommu: translation on for ",
    "[info ] nvme: up ",
    "[info ] fat32: mounted ",
    "[info ] keyboard: PCI interrupt isolation keeps every io-apic line masked; polling",
    "[info ] keyboard: polled every 10 ms",
    USER_CALL,
];

const NO_X2APIC_REQUIRED: &[&str] = &[
    STRICT_DMA_HANDOFF,
    SHIPPED_ISOLATION,
    "[warn ] local apic: this processor has no x2apic mode, and xapic needs page tables; interrupts stay off",
    "[info ] tmpfs: 1 file(s) and 1 directory(ies) at the root",
    "[info ] devfs: storage is a block device",
    "[info ] devfs: console is a character device",
    "[info ] cpu: protections wp nx",
    "[warn ] cpu: extended state unavailable; ring 3 stays closed",
    "[warn ] user: no program: maskable interrupts are not admitted; ring 3 is closed",
];

// The first candidate's clean retirement proves the refusal path returns its lease exactly.
const UNISOLATED_REQUIRED: &[&str] = &[
    STRICT_DMA_HANDOFF,
    SHIPPED_ISOLATION,
    "[warn ] storage: no DMA isolation",
    NVME_FALLBACK,
    "[info ] nvme: no usable controller",
];

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
        required: &'static [&'static str],
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
        self.records.observe(line);
        self.settled |= line.starts_with(SETTLED);
        self.key_consumed |= self.key_injected && line.starts_with(KEY_CONSUMED);
        Ok(())
    }

    fn until_settled(&mut self) -> Result<()> {
        let deadline = Instant::now() + BOOT_TIMEOUT;
        while !self.settled {
            match self.machine.next(deadline)? {
                Event::Line(line) => self.observe(&line)?,
                Event::Exited(status) => bail!("the {} kernel exited ({status})", self.kernel),
                Event::TimedOut => bail!("the {} kernel never reached {SETTLED:?}", self.kernel),
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
    let command = Installation::locate()?.command(run, Remapping::Present)?;
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
    let mut command = Installation::locate()?.command(run, Remapping::Present)?;
    command.args(["-cpu", DEGRADED_CPU]);
    println!("[xtask] booting the shipped kernel with no x2apic");
    settle(Boot::launch(
        command,
        Control::None,
        "no-x2apic",
        NO_X2APIC_REQUIRED,
    )?)?;
    println!("[xtask] optional CPU protection fallback remains diagnostic-only");

    let command = Installation::locate()?.command(run, Remapping::Absent)?;
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
