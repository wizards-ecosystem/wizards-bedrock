//! QEMU `isa-debug-exit`: a port write ends the test as `(value << 1) | 1`.
//!
//! Compiled only for `selftest`. A missing device ignores the write.

use wost::arch::current::port::{self, Port};

/// Written when every check passed. QEMU then exits with 33.
const PASSED: u32 = 0x10;

/// Written when any check failed. QEMU then exits with 35.
const FAILED: u32 = 0x11;

/// Tell the harness whether the run passed. Under QEMU with the device present this terminates the
/// emulator and never returns.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "the harness verdict is one pass-or-fail bit"
)]
pub(crate) fn report(passed: bool) {
    if let Some(mut device) = port::debug_exit() {
        Port::write(&mut device, if passed { PASSED } else { FAILED });
    }
}
