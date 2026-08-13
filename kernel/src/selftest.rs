//! Boot-time self-checks.
//!
//! The kernel cannot host `cargo test`: it is a `#![no_std] #![no_main]` binary,
//! and the in-QEMU test harnesses need nightly. So the checks that need a real
//! machine run here instead, on a kernel built with the `selftest` feature, and
//! report their verdict through [`debug_exit`](crate::arch::current::debug_exit).
//!
//! Everything here must be checkable *on the machine*. Anything that is pure
//! arithmetic belongs in a `libs/` crate with ordinary unit tests instead.
//!
//! Checks run against a fully booted kernel, so anything the boot path already
//! asserts is not repeated here — a rejected base revision panics long before
//! this, which the harness sees as a missing verdict.

use wost::arch::current::port;
use wost::boot::{FramebufferInfo, Rgb};
use wost::sync::SpinLock;

use crate::arch::current::debug_exit;
use crate::arch::current::uart::{self, UartError};
use crate::{dev, error, info};

/// Tally of what has been checked so far.
struct Report {
    passed: usize,
    failed: usize,
}

impl Report {
    const fn new() -> Self {
        Self {
            passed: 0,
            failed: 0,
        }
    }

    fn check(&mut self, name: &str, holds: bool) {
        if holds {
            self.passed += 1;
            info!("selftest: {name}");
        } else {
            self.failed += 1;
            error!("selftest: {name} FAILED");
        }
    }
}

/// Run every check and hand the verdict to the harness.
pub(crate) fn run(serial: Result<(), UartError>) {
    let mut report = Report::new();
    let geometry = dev::console_geometry();

    report.check("serial console came up", serial.is_ok());
    report.check("framebuffer console came up", geometry.is_some());

    if let Some(geometry) = geometry {
        report.check("framebuffer geometry is sane", geometry_is_sane(geometry));
        report.check(
            "colour channels do not overlap",
            channels_are_disjoint(geometry),
        );
    }

    report.check(
        "a claimed port cannot be claimed twice",
        ports_are_exclusive(),
    );
    report.check("a held lock excludes another holder", lock_excludes());

    let passed = report.failed == 0;
    if passed {
        info!("selftest: {} checks passed", report.passed);
    } else {
        error!(
            "selftest: {} passed, {} FAILED",
            report.passed, report.failed
        );
    }
    debug_exit::report(passed);
}

/// A row must be wide enough to hold the pixels the bootloader says are in it.
fn geometry_is_sane(geometry: FramebufferInfo) -> bool {
    geometry.width > 0
        && geometry.height > 0
        && geometry.bytes_per_pixel > 0
        && geometry.pitch >= geometry.width * geometry.bytes_per_pixel
}

/// The bootloader's masks must place red, green and blue in disjoint bits.
///
/// Overlapping masks would not fail loudly — text would simply render in the
/// wrong colour — so this is worth asserting against the format we were handed
/// rather than the one we expect.
fn channels_are_disjoint(geometry: FramebufferInfo) -> bool {
    let pack = |red, green, blue| geometry.format.pack(Rgb { red, green, blue });
    let (red, green, blue) = (
        pack(u8::MAX, 0, 0),
        pack(0, u8::MAX, 0),
        pack(0, 0, u8::MAX),
    );

    red != 0 && green != 0 && blue != 0 && red & green == 0 && green & blue == 0 && red & blue == 0
}

/// COM1 is already owned by the serial driver, so a second claim must be refused.
fn ports_are_exclusive() -> bool {
    port::claim_block::<8>(uart::COM1).is_none()
}

fn lock_excludes() -> bool {
    let lock = SpinLock::new(0u32);
    let held = lock.lock();
    let refused = lock.try_lock().is_none();
    drop(held);
    refused && lock.try_lock().is_some()
}
