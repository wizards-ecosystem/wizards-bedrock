//! Panic handling.
//!
//! The handler reports on both consoles and halts. There is no unwind.

use core::fmt::Write;
use core::panic::PanicInfo;

use panic_record::write_record;
use wost::arch::current::cpu;

use crate::{dev, theme, thread};

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    wost::panic::terminate(info, report)
}

fn report(info: &PanicInfo<'_>) -> ! {
    // The thread is named because a panic in one of several is otherwise a report with no subject:
    // every thread runs the same kernel.
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
