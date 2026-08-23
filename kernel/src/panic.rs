//! Panic handling.
//!
//! A panic is terminal: there is no unwinding (`panic = "abort"`) and no
//! supervisor to restart anything yet, so the handler reports what happened
//! through both consoles and stops the machine.

use core::fmt::Write;
use core::panic::PanicInfo;

use wost::arch::current::cpu;

use crate::{dev, error, input, theme, thread};

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    let Some(token) = wost::panic::enter() else {
        cpu::halt_forever();
    };
    // Everything, not just the consoles: a fault is not masked by the
    // interrupt flag, so any lock in the kernel can be held by the code this
    // handler interrupted, and a report that deadlocks is no report.
    wost::panic::reclaim(&token);
    dev::force_unlock(&token);
    input::force_unlock(&token);
    input::force_unlock_watch(&token);
    thread::force_unlock(&token);

    // The thread is named because a panic in one of several is otherwise a
    // report with no subject: every thread runs the same kernel.
    let thread = thread::current_name();
    error!("KERNEL PANIC on thread '{thread}': {info}");

    dev::with_console(|console| {
        console.reset(theme::PANIC);
        console.write_line("");
        console.write_line("  KERNEL PANIC");
        console.write_line("");
        let _ = writeln!(console, "  thread '{thread}'");
        let _ = writeln!(console, "  {info}");
    });

    cpu::halt_forever();
}
