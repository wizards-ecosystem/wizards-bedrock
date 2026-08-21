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

use core::hint::spin_loop;

use wost::arch::current::cpu::{self, Protections};
use wost::arch::current::{apic, interrupt, pic, port, segment};
use wost::boot::{FramebufferInfo, Rgb};
use wost::sync::{Once, SpinLock};
use x86_tables::exception;

use crate::arch::current::debug_exit;
use crate::arch::current::trap::{self, Record};
use crate::arch::current::uart::{self, UartError};
use crate::arch::current::vector;
use crate::{dev, error, info, input, time};

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
    report.check("a one-shot latch admits one caller", latch_admits_one());

    report.check(
        "the code selector is the one we installed",
        segment::code_selector() == segment::KERNEL_CODE,
    );
    report.check(
        "the task register holds our segment",
        segment::task_register() == segment::TASK_STATE,
    );
    report.check(
        "the interrupt table the CPU holds is ours",
        interrupt::table_is_loaded(),
    );

    let before = trap::last_breakpoint();
    let stack_pointer = cpu::stack_pointer();
    interrupt::breakpoint();
    let taken = trap::last_breakpoint();

    report.check(
        "a breakpoint is delivered and resumes",
        taken.count == before.count + 1,
    );
    report.check(
        "the saved frame is the frame the CPU pushed",
        frame_is_consistent(taken, stack_pointer),
    );

    report.check(
        "every protection the CPU supports is enabled",
        Protections::enabled() == Protections::supported(),
    );

    report.check(
        "the legacy controllers are masked",
        pic::masks() == Some((MASKED, MASKED)),
    );
    report.check("the local apic is software enabled", apic::is_enabled());
    report.check("this cpu is accepting interrupts", interrupt::are_enabled());
    report.check(
        "a lock excludes interrupts and gives them back",
        a_lock_excludes_interrupts(),
    );

    // Two in a row, because one only proves the interrupt arrived. The APIC
    // will not deliver a second of the same priority class until the first is
    // dismissed, so the second is what proves the handler's end-of-interrupt.
    let before = trap::last_ping();
    apic::send_self_interrupt(vector::PING);
    report.check(
        "a self-interrupt is delivered",
        ping_arrives(before.count + 1),
    );

    let taken = trap::last_ping();
    apic::send_self_interrupt(vector::PING);
    report.check(
        "the interrupt is dismissed, so the next one arrives",
        ping_arrives(before.count + 2),
    );
    report.check(
        "the interrupted code was taking interrupts",
        ping_is_consistent(taken),
    );

    report.check(
        "the timer entry is periodic and unmasked",
        timer_entry_is_programmed(),
    );
    report.check("the calibrated rate is believable", rate_is_believable());

    let ticks = time::ticks();
    report.check("the tick counter advances", ticks_reach(ticks + 1));
    // Same argument as the ping pair: the APIC will not deliver a second tick
    // until the first is dismissed, so the second proves the handler's
    // end-of-interrupt.
    report.check(
        "a tick is dismissed, so the next one arrives",
        ticks_reach(ticks + 2),
    );

    let sweep = sweep_uptime();
    report.check("uptime never runs backwards", sweep.monotonic);
    report.check("uptime resolves finer than a tick", sweep.interpolated);
    report.check(
        "uptime agrees with the tick counter",
        uptime_matches_ticks(),
    );

    report.check("the keyboard controller came up", input::is_ready());
    report.check(
        "the controller raises no interrupts and does not translate",
        keyboard_is_silent_and_raw(),
    );
    report.check(
        "an echo to the keyboard comes back",
        input::echo_roundtrips(),
    );
    report.check(
        "the keyboard is speaking scancode set two",
        input::scancode_set() == Some(2),
    );

    report.check(
        "no spurious interrupt has been delivered",
        trap::spurious_count() == 0,
    );

    // Last, because reading the error register clears it.
    report.check(
        "the local apic has recorded no error",
        apic::error_status() == 0,
    );

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

fn latch_admits_one() -> bool {
    let latch = Once::new();
    !latch.is_claimed() && latch.claim() && !latch.claim() && latch.is_claimed()
}

/// Every input line on both legacy controllers, disconnected.
const MASKED: u8 = 0xFF;

/// Holding a lock must exclude interrupts, releasing it must give them back,
/// and a nested lock must not give them back early — the last being the one
/// that a naive "enable on drop" gets wrong, and the one that would show up as
/// an interrupt arriving in the middle of a critical section months later.
///
/// Only meaningful with interrupts already on, which by this point they are.
fn a_lock_excludes_interrupts() -> bool {
    let outer = SpinLock::new(0u32);
    let inner = SpinLock::new(0u32);

    let before = interrupt::are_enabled();
    let outer_guard = outer.lock();
    let excluded = !interrupt::are_enabled();

    let inner_guard = inner.lock();
    drop(inner_guard);
    let still_excluded = !interrupt::are_enabled();

    drop(outer_guard);
    before && excluded && still_excluded && interrupt::are_enabled()
}

/// Spin until the ping count reaches `target`, giving up rather than hanging
/// the harness on an interrupt that is never going to arrive.
///
/// Generous, because it is measuring hardware that has already been told to do
/// something: a delivery this does not see within a million reads is not slow,
/// it is absent.
fn ping_arrives(target: usize) -> bool {
    const ATTEMPTS: u32 = 1_000_000;

    (0..ATTEMPTS).any(|_| {
        spin_loop();
        trap::last_ping().count >= target
    })
}

/// The hardware's own record of the timer: unmasked, periodic, our vector.
fn timer_entry_is_programmed() -> bool {
    let entry = apic::timer_entry();
    !entry.is_masked() && entry.is_periodic() && entry.vector() == vector::TIMER
}

/// The measured rate, against bounds restated here rather than shared with
/// the code under test: a megahertz to fifty gigahertz covers every machine
/// this could believably boot on, and calibration having declined reads as
/// zero, which fails too.
fn rate_is_believable() -> bool {
    (1_000_000..=50_000_000_000).contains(&time::divided_hz())
}

/// Spin until the tick count reaches `target`, giving up rather than hanging
/// the harness on a timer that is never going to fire.
///
/// Sized in reads rather than time: tens of millions of them span many 10 ms
/// ticks on anything that can boot this kernel, and cost seconds, not
/// minutes, when the timer is dead.
fn ticks_reach(target: u64) -> bool {
    const ATTEMPTS: u32 = 50_000_000;

    (0..ATTEMPTS).any(|_| {
        spin_loop();
        time::ticks() >= target
    })
}

/// What a continuous read of the clock across several ticks saw.
struct UptimeSweep {
    monotonic: bool,
    interpolated: bool,
}

/// Read the clock as fast as it can be read until three ticks have passed.
///
/// Monotonicity is the property under test; interpolation is observed on the
/// side, as at least one reading that is not a whole tick. The very first
/// reading is taken while holding a lock — the case the clamp in `time`
/// exists for, because the countdown can reload under a clear interrupt flag
/// while the tick it announces stays pending.
fn sweep_uptime() -> UptimeSweep {
    const ATTEMPTS: u32 = 20_000_000;
    const TICK_SPAN: u64 = 3;

    let per_tick = u64::from(time::MS_PER_TICK);
    let start = time::ticks();
    let mut previous = 0;
    let mut sweep = UptimeSweep {
        monotonic: true,
        interpolated: false,
    };

    for attempt in 0..ATTEMPTS {
        let sample = if attempt == 0 {
            let lock = SpinLock::new(0u32);
            let held = lock.lock();
            let sample = time::uptime_ms();
            drop(held);
            sample
        } else {
            time::uptime_ms()
        };

        if sample < previous {
            sweep.monotonic = false;
        }
        if !sample.is_multiple_of(per_tick) {
            sweep.interpolated = true;
        }
        previous = sample;

        if time::ticks() >= start + TICK_SPAN {
            break;
        }
    }
    sweep
}

/// The two clocks agree: an uptime reading lands between the tick counts
/// taken on either side of it, each converted to the milliseconds it stands
/// for, with one tick of slack for the read in flight.
///
/// Checked only once twenty ticks have passed. The bracket allows one tick of
/// slack, so a conversion off by even a millisecond per tick hides inside it
/// until the error has had ten ticks to accumulate — breaking this check once
/// proved exactly that, when a fresh-booted clock let a wrong ratio through.
fn uptime_matches_ticks() -> bool {
    const SETTLE_TICKS: u64 = 20;

    if !ticks_reach(SETTLE_TICKS) {
        return false;
    }
    let per_tick = u64::from(time::MS_PER_TICK);
    let floor = time::ticks() * per_tick;
    let uptime = time::uptime_ms();
    let ceiling = (time::ticks() + 1) * per_tick;
    floor <= uptime && uptime <= ceiling
}

/// The configuration byte, against bits restated from the 8042 datasheet
/// rather than shared with the driver: both interrupt enables and the set 1
/// translation shim must be off, read back live from the hardware — the
/// self-test's own reset of this byte is the quirk the driver rewrites it
/// after, and this is the check that would catch the rewrite going missing.
fn keyboard_is_silent_and_raw() -> bool {
    const FIRST_IRQ: u8 = 1 << 0;
    const SECOND_IRQ: u8 = 1 << 1;
    const TRANSLATE: u8 = 1 << 6;

    match input::controller_config() {
        Some(byte) => byte & (FIRST_IRQ | SECOND_IRQ | TRANSLATE) == 0,
        None => false,
    }
}

/// The frame a delivered interrupt saved, checked the way `frame_is_consistent`
/// checks a breakpoint's — except that this one can also assert something a
/// trap cannot: the interrupted code had the interrupt flag set, which is the
/// only direct evidence that this arrived through the flag rather than as a
/// software `int`.
fn ping_is_consistent(ping: Record) -> bool {
    /// `RFLAGS` bit 9.
    const INTERRUPT_FLAG: u64 = 1 << 9;

    ping.vector == u64::from(vector::PING)
        && ping.error_code == 0
        && ping.code_selector == u64::from(segment::KERNEL_CODE.bits())
        && ping.flags & INTERRUPT_FLAG != 0
}

/// The interrupt frame is the one place a struct definition and an assembly
/// listing have to agree, and the compiler cannot check it. If the pushes and
/// the fields drift apart, these values are what turn to nonsense.
///
/// `stack_pointer` is the caller's, taken immediately before the breakpoint: the
/// saved one belongs to a frame or two deeper, so it is checked for proximity
/// rather than equality.
fn frame_is_consistent(breakpoint: Record, stack_pointer: u64) -> bool {
    /// Bit 1 of the flags register reads as one on every x86 ever made.
    const RESERVED_FLAG: u64 = 1 << 1;

    /// How far the saved stack pointer may be from the one measured beside it.
    const STACK_SLACK: u64 = 4096;

    breakpoint.vector == u64::from(exception::BREAKPOINT)
        && breakpoint.error_code == 0
        && breakpoint.code_selector == u64::from(segment::KERNEL_CODE.bits())
        && breakpoint.flags & RESERVED_FLAG != 0
        && breakpoint.stack_pointer.abs_diff(stack_pointer) < STACK_SLACK
}
