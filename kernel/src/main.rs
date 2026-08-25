//! The Wizard's OS kernel — entry point.
//!
//! Booted by Limine (base revision 4, D-42) on x86_64, higher-half. Every privileged
//! operation goes through `wost`; this crate contains no hand-written `unsafe`.
//! `deny` rather than `forbid` because [`wost::entry!`] expands its own
//! `#[allow(unsafe_code)]` glue here — see D-21.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

/// The heap `wost::mm::heap` provides. Nothing in the kernel may allocate
/// before `mem::init` has run, which is why the boot order in `kernel_main`
/// puts it before everything that might.
extern crate alloc;

mod acpi;
mod arch;
mod banner;
mod console;
mod dev;
// Ungated at the Phase 5 close. The order the charter forced was handles
// before objects before a process to belong to, so these were built and proven
// behind the selftest feature first — and the gate comes off with `spawn`,
// which the syscall path now reaches unconditionally (D-116).
mod handle;
mod input;
mod log;
mod mem;
mod object;
mod panic;
mod queue;
#[cfg(feature = "selftest")]
mod selftest;
mod theme;
mod thread;
mod time;
mod user;
mod wait;

use core::fmt::Write;

use crate::arch::current::uart::UartError;
use wost::arch::current::{apic, cpu, interrupt, pic, segment, task, xstate};
use wost::boot::BootInfo;
use wost::sync::SpinLock;

use crate::arch::current::{ioapic, vector};

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
    let protections = cpu::enable_protections();
    info!("cpu: protections {protections}");

    // Before any thread but this one exists, because every switch saves and
    // restores from here on (D-108).
    match xstate::init() {
        Some(extended) => info!(
            "cpu: extended state {} bytes, xcr0 {:#x}{}",
            extended.bytes,
            extended.mask,
            if extended.compacted {
                ", compacted"
            } else {
                ""
            }
        ),
        None => info!("cpu: no extended state to save"),
    }

    // Before anything can enter ring 3, and only where the instruction exists:
    // `AC` survives interrupt delivery and is writable at CPL 3, so a ring-3
    // thread can otherwise hand the kernel a trap with SMAP already off
    // (D-101).
    if protections.smap {
        interrupt::arm_access_clear();
    }

    // After the fault path, because the first write through the direct map is
    // the first time this kernel touches an address the bootloader chose rather
    // than the linker — and before everything else, because from here on
    // anything may want a frame.
    mem::init(boot.memory_map, boot.direct_map);
    let routing = mem::init_paging(boot.memory_map, boot.direct_map, boot.image, boot.rsdp);

    // The boot thread leaves the bootloader's stack here, which is the only way
    // to guard it: that stack has nothing below it and lives inside the direct
    // map, which is two-megabyte on purpose (D-92). Everything after this point
    // runs in `boot_continue`, on a stack with an unmapped page beneath it.
    *HANDOFF.lock_leaf() = Some(Handoff {
        serial,
        routing,
        module: boot.module,
    });
    match mem::map_boot_stack() {
        Some(stack) => task::continue_on_stack(stack, boot_continue, 0),
        None => {
            warn!("boot: staying on the bootloader's stack, which has no guard");
            boot_continue(0)
        }
    }
}

/// What the boot thread carries across the change of stack.
///
/// A static rather than an argument because the old stack is abandoned: a
/// pointer into it would be a pointer into memory nothing owns any more.
struct Handoff {
    serial: Result<(), UartError>,
    routing: Option<mem::Routing>,
    /// The initrd, which the boot thread reads before it changes stacks and
    /// the loader reads long after.
    module: Result<&'static [u8], wost::boot::HandoffError>,
}

static HANDOFF: SpinLock<Option<Handoff>> = SpinLock::new(None);

/// The rest of the boot, on a guarded stack.
///
/// # Panics
///
/// If the handoff is missing, which would mean this ran twice.
extern "C" fn boot_continue(_arg: usize) -> ! {
    let Some(Handoff {
        serial,
        routing,
        module,
    }) = HANDOFF.lock_leaf().take()
    else {
        panic!("the boot handoff is gone; boot_continue ran twice");
    };

    // The heap exists from here, and the console is its first consumer: a
    // shadow in RAM is what stops every scroll reading uncached device memory
    // back (D-48).
    dev::attach_back_buffer();

    start_interrupts();
    input::init();
    route_keyboard(routing);
    input::start_poller();

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

    user::keep(module);

    #[cfg(feature = "selftest")]
    selftest::run(serial);

    // After interrupts are on, because a halting thread needs them to be
    // woken. Idempotent: the self-checks start it earlier, to watch the
    // machine on both sides of it.
    thread::start_idle();

    // And the shipped kernel runs the program, which until D-116 only the
    // checks did. A kernel that has userspace and never enters it has not
    // shipped userspace: the loader, the address space, the handle table, the
    // ring and the gate were all dead code in the image people actually boot.
    //
    // Reported rather than returned: there is exactly one caller and nothing
    // it could do differently. A refusal is a line in the log and a kernel
    // that goes on reading the keyboard, which is the behaviour a program
    // failing to start should have.
    match user::run() {
        Ok(program) => info!("user: program running in slot {}", program.slot),
        Err(reason) => warn!("user: no program: {reason}"),
    }
    info!(
        "kernel: reading the keyboard, uptime {} ms",
        time::uptime_ms()
    );
    echo_keystrokes()
}

/// The boot thread's last job: block on the keyboard, and draw what arrives.
///
/// Nothing polls and nothing spins: this thread is not runnable until a
/// keystroke wakes it (D-61), the idle thread halts meanwhile, and no timer
/// is armed — so a kernel at a prompt takes one interrupt per keystroke.
///
/// Drawing is here rather than in the handler because a glyph can cost a
/// scroll (D-63).
fn echo_keystrokes() -> ! {
    let mut context = thread::ThreadContext::require();
    loop {
        let keystroke = input::read_key(&mut context);
        debug!("key: {:?} {:?}", keystroke.event.key, keystroke.event.state);
        if let Some(character) = keystroke.character {
            dev::with_console(|console| {
                let _ = console.write_char(character);
            });
        }
    }
}

/// Bring up the interrupt controller, start the clock, and accept interrupts.
///
/// Each step is a precondition for the next, and every one of them can decline:
/// a kernel that cannot take interrupts is worth far more than one that takes
/// them into a controller it never configured. In order — stop the legacy
/// controllers delivering anything, give the local APIC its vectors and turn
/// it on, measure the clocks, and only then set the flag. Calibration sits
/// where it does for both of its neighbours: it needs the APIC's registers
/// answering, and running before the `sti` is what makes the measurement
/// unpreemptible. It arms nothing — the timer is the scheduler's to arm, and
/// nothing has asked it for anything yet. Alone among the steps, its failure
/// vetoes nothing: a kernel without a clock still wants its keyboard.
fn start_interrupts() {
    if let Err(reason) = pic::silence() {
        warn!("legacy controllers not silenced: {reason}; interrupts stay off");
        return;
    }
    info!("pic: legacy controllers remapped and masked");

    // One MSR write, at boot, where it is trivial (D-81). The report is the
    // point as much as the write: what a machine claims about speculation
    // varies enormously in 2026, and this is the only place it is visible.
    let speculation = cpu::enable_speculation_controls();
    info!("cpu: {speculation}");

    // The flush on the way back to ring 3, armed unless the processor says it
    // is unnecessary (D-98). Not gated on `MD_CLEAR`: `verw` exists on every
    // x86-64, and that bit says only whether it *also* clears the buffers — so
    // a machine whose capability word cannot be read gets the instruction
    // rather than the assumption, which is the direction to be wrong in.
    // `MDS_NO` is the one claim that makes it pointless.
    if speculation.mds_no() {
        info!("cpu: no buffer clear on return to ring 3; the processor reports mds_no");
    } else {
        interrupt::enable_buffer_clear();
        info!(
            "cpu: buffers cleared on every return to ring 3{}",
            if cpu::has_buffer_clear() {
                ""
            } else {
                " (md_clear unreported, so the flush may buy nothing here)"
            }
        );
    }

    match apic::init(vector::LOCAL_APIC) {
        Ok(local) => info!("apic: {local}"),
        Err(reason) => {
            warn!("local apic: {reason}; interrupts stay off");
            return;
        }
    }

    match time::init() {
        Ok(report) => {
            info!("time: {report}");
            // Loud, because it is the one property of the clock this kernel
            // cannot check for itself: `cpuid` either promises the counter
            // runs at a constant rate or it does not, and a machine that
            // declines the promise may still be throttling underneath us.
            if !report.counter_is_invariant() {
                warn!(
                    "time: the counter is not advertised invariant; the clock is only as steady as this machine's power management"
                );
            }
        }
        Err(reason) => warn!("clock: {reason}; uptime stays at zero, nothing preempts"),
    }

    interrupt::enable();
    info!("cpu: accepting interrupts");
}

/// Route the keyboard's line through the I/O APIC, so keystrokes interrupt.
///
/// Declines rather than fails: a kernel whose firmware described no I/O APIC,
/// or whose registers would not answer, keeps the polled path Phase 2 left
/// behind (D-37), which `input::start_poller` then gives a thread to.
fn route_keyboard(routing: Option<mem::Routing>) {
    /// The legacy line the keyboard controller has always used.
    const KEYBOARD_IRQ: u8 = 1;

    let Some(mut routing) = routing else {
        info!("keyboard: no io apic; a poller thread drains it instead");
        return;
    };

    let gsi = routing.interrupts.gsi_for(KEYBOARD_IRQ);
    if !routing.io_apic.handles(gsi) {
        warn!("keyboard: gsi {gsi} is not this controller's; staying polled");
        return;
    }

    let wiring = match routing.interrupts.flags_for(KEYBOARD_IRQ) {
        0 => ioapic::Wiring::isa_default(),
        flags => ioapic::Wiring::from_flags(flags),
    };
    if let Err(reason) = routing.io_apic.route(gsi, vector::KEYBOARD, wiring) {
        warn!("keyboard: gsi {gsi} not routed: {reason}; staying polled");
        return;
    }

    // The line is routed before the controller is allowed to raise it: the
    // other order would let a byte already waiting fire into a vector the
    // I/O APIC had not yet been told about.
    if !input::enable_interrupts() {
        warn!("keyboard: controller would not raise its line; staying polled");
        return;
    }
    input::take_interrupts();
    dev::keep_io_apic(routing.io_apic, gsi);
    info!(
        "keyboard: irq {KEYBOARD_IRQ} on gsi {gsi} routed to vector {:#x}",
        vector::KEYBOARD
    );
}
