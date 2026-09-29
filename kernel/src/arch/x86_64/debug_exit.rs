//! QEMU's `isa-debug-exit` device.
//!
//! Writing to it makes QEMU terminate with `(value << 1) | 1`, which is how a
//! kernel running under test reports its verdict to the harness. Present only
//! when QEMU is started with `-device isa-debug-exit`, so this is compiled in
//! with the `selftest` feature and never ships in a real image.
//!
//! Nothing here is privileged: a claimed `Port` writes in safe code.

use wost::arch::current::port::{self, Port};

/// Written when every check passed. QEMU then exits with 33.
const PASSED: u32 = 0x10;

/// Written when any check failed. QEMU then exits with 35.
const FAILED: u32 = 0x11;

/// Tell the harness whether the run passed.
///
/// Under QEMU with the device present this terminates the emulator and never
/// returns. Anywhere else the write goes nowhere and the caller carries on, so
/// a self-test kernel booted on real hardware still reaches its idle loop.
pub(crate) fn report(passed: bool) {
    if let Some(mut device) = port::debug_exit() {
        Port::write(&mut device, if passed { PASSED } else { FAILED });
    }
}
