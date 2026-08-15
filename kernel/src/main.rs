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
mod log;
mod panic;
#[cfg(feature = "selftest")]
mod selftest;
mod theme;

use core::fmt::Write;

use wost::arch::current::{cpu, segment};
use wost::boot::BootInfo;

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

    info!("kernel: entering idle loop");
    cpu::halt_forever()
}
