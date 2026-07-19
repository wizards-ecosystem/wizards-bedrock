//! The Wizard's OS kernel — entry point.
//!
//! Booted by Limine (base revision 3) on x86_64, higher-half.

#![no_std]
#![no_main]

mod framebuffer;
mod serial;

use core::fmt::Write;

use limine::BaseRevision;
use limine::request::{FramebufferRequest, RequestsEndMarker, RequestsStartMarker};

#[used]
#[unsafe(link_section = ".requests")]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(link_section = ".requests_start_marker")]
static _REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".requests_end_marker")]
static _REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    serial::init();
    serial_println!("");
    serial_println!("The Wizard's OS awakens.");

    assert!(BASE_REVISION.is_supported());
    serial_println!("limine: base revision supported");

    if let Some(response) = FRAMEBUFFER_REQUEST.get_response() {
        if let Some(fb) = response.framebuffers().next() {
            serial_println!(
                "framebuffer: {}x{} bpp={} pitch={}",
                fb.width(),
                fb.height(),
                fb.bpp(),
                fb.pitch()
            );
            let mut console = framebuffer::Console::new(&fb);
            console.write_line("");
            console.write_line("  The Wizard's OS");
            console.write_line("  ~~~~~~~~~~~~~~~");
            console.write_line("");
            console.write_line("  first light: hello from ring 0");
            console.write_line("  boot: Limine / UEFI / x86_64 higher half");
        }
    } else {
        serial_println!("framebuffer: no response from Limine");
    }

    serial_println!("kmain: entering idle loop");
    hcf();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = writeln!(serial::Writer, "KERNEL PANIC: {info}");
    hcf();
}

/// Halt forever.
fn hcf() -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}
