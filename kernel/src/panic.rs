//! Panic handling: with no unwinding (`panic = "abort"`) and no supervisor yet, a panic is
//! terminal, so the handler reports through both consoles and stops the machine.

use core::fmt::Write;
use core::panic::PanicInfo;

use panic_record::write_record;
use wost::arch::current::cpu;

use crate::{dev, theme, thread};

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    wost::panic::terminate(info, report)
}

/// The thread is named because every thread runs the same kernel, so a report needs a subject.
fn report(info: &PanicInfo<'_>) -> ! {
    let thread = thread::current_name();
    dev::try_with_uart(|uart| {
        let _ = write_record(uart, thread, info);
    });

    dev::try_with_console(|console| {
        console.reset(theme::PANIC);
        console.write_line("");
        console.write_line("  KERNEL PANIC");
        console.write_line("");
        let _ = writeln!(console, "  thread '{thread}'");
        let _ = writeln!(console, "  {info}");
    });

    cpu::halt_forever();
}
