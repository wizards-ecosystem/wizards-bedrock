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
mod theme;

use wost::arch::current::cpu;
use wost::boot::BootInfo;

wost::entry!(kernel_main);

fn kernel_main(boot: BootInfo) -> ! {
    let serial = dev::init_uart();
    info!("The Wizard's OS awakens.");

    assert!(
        boot.base_revision_supported,
        "bootloader rejected our Limine base revision"
    );
    info!("limine: base revision supported");

    match boot.framebuffer {
        Ok(framebuffer) => {
            dev::init_console(framebuffer);
            dev::with_console(|console| {
                for line in banner::LINES {
                    console.write_line(line);
                }
                // The one message that cannot go to the log it is about.
                if !serial {
                    console.write_line("");
                    console.write_line("  no serial console found — diagnostics are lost");
                }
            });
        }
        Err(reason) => warn!("framebuffer: {reason}"),
    }

    info!("kernel: entering idle loop");
    cpu::halt_forever();
}
