//! The Wizard's OS kernel — entry point.
//!
//! Booted by Limine (base revision 3) on x86_64, higher-half. Every privileged
//! operation goes through `wost`; this crate contains no hand-written `unsafe`.
//! `deny` rather than `forbid` because [`wost::entry!`] expands its own
//! `#[allow(unsafe_code)]` glue here — see D-21.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

mod arch;
mod banner;
mod console;
mod dev;
mod input;
mod log;
mod panic;
#[cfg(feature = "selftest")]
mod selftest;
mod theme;
mod time;

use core::fmt::Write;

use wost::arch::current::{apic, cpu, interrupt, pic, segment};
use wost::boot::BootInfo;

use crate::arch::current::vector;

wost::entry!(kernel_main);

fn kernel_main(boot: BootInfo) -> ! {
    let serial = dev::init_uart();
    info!("The Wizard's OS awakens.");

    match boot.framebuffer {
        Ok(framebuffer) => dev::init_console(framebuffer),
        Err(reason) => warn!("framebuffer: {reason}"),
    }

    // Checked only once both consoles are up, so that a machine with neither a
    // serial cable nor a supported bootloader still shows why it stopped.
    assert!(
        boot.base_revision_supported,
        "bootloader rejected our Limine base revision"
    );
    info!("limine: base revision supported");

    // Order matters: the fault path exists before anything that can fault, so a
    // wrong assumption about the bootloader's page tables prints a report
    // instead of resetting the machine. See `docs/006-cpu-fundamentals.md`.
    segment::init();
    arch::current::trap::init();
    info!("cpu: descriptor tables installed");
    info!("cpu: protections {}", cpu::enable_protections());

    start_interrupts();
    input::init();

    dev::with_console(|console| {
        for line in banner::LINES {
            console.write_line(line);
        }
        // The one message that cannot go to the log it is about.
        if let Err(reason) = serial {
            console.write_line("");
            let _ = writeln!(console, "  no serial log: {reason}");
        }
    });

    #[cfg(feature = "selftest")]
    selftest::run(serial);

    info!(
        "kernel: entering idle loop, uptime {} ms",
        time::uptime_ms()
    );
    idle()
}

/// Halt until something happens, do the work it brought, halt again.
///
/// Everything an interrupt wakes gets its turn between the halts — today
/// that is the keyboard, drained on every tick. With the flag never set —
/// interrupts declined at boot — the first halt is permanent, which is the
/// same idle this kernel had before it could take interrupts.
fn idle() -> ! {
    loop {
        cpu::halt();
        input::poll();
    }
}

/// Bring up the interrupt controller, start the clock, and accept interrupts.
///
/// Each step is a precondition for the next, and every one of them can decline:
/// a kernel that cannot take interrupts is worth far more than one that takes
/// them into a controller it never configured. In order — stop the legacy
/// controllers delivering anything, give the local APIC its vectors and turn
/// it on, calibrate and arm the timer, and only then set the flag. The timer
/// sits where it does for both of its neighbours: it needs the APIC's
/// registers answering, and running before the `sti` makes the measurement
/// unpreemptible and holds the first tick pending until the kernel is ready
/// to take it. Alone among the steps, its failure vetoes nothing — a kernel
/// without a clock still wants its keyboard.
fn start_interrupts() {
    if let Err(reason) = pic::silence() {
        warn!("legacy controllers not silenced: {reason}; interrupts stay off");
        return;
    }
    info!("pic: legacy controllers remapped and masked");

    match apic::init(vector::LOCAL_APIC) {
        Ok(local) => info!("apic: {local}"),
        Err(reason) => {
            warn!("local apic: {reason}; interrupts stay off");
            return;
        }
    }

    match time::init() {
        Ok(report) => info!("time: {report}"),
        Err(reason) => warn!("timer: {reason}; uptime stays at zero"),
    }

    interrupt::enable();
    info!("cpu: accepting interrupts");
}
