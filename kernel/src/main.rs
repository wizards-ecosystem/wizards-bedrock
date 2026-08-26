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
mod fs;
mod handle;
mod input;
mod log;
mod mem;
mod nvme;
mod object;
mod panic;
mod pci;
mod queue;
#[cfg(feature = "selftest")]
mod selftest;
mod theme;
mod thread;
mod time;
mod timepage;
mod user;
mod vfs;
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

    // The page exists whether or not the clock does, because the ABI says it
    // is there: a program that reads `TIME` on a machine with no clock must
    // find `MODE_NONE` and fall back, not take a page fault (D-166).
    if timepage::init(time::counter_hz()) {
        let mode = if time::is_calibrated() {
            timepage::MODE_COUNTER
        } else {
            timepage::MODE_NONE
        };
        timepage::publish(mode, time::origin_ticks(), time::conversion());
        info!(
            "time: published at {:#x}, {}",
            timepage::TIME,
            if mode == timepage::MODE_COUNTER {
                "readable without a call"
            } else {
                "revoked; readers must ask"
            }
        );
    } else {
        warn!("time: no page to publish the clock in; every read is a call");
    }

    survey_the_bus();

    interrupt::enable();
    info!("cpu: accepting interrupts");
}

/// Claim configuration space and say what is on the bus.
///
/// Declines rather than fails: a machine whose configuration ports are already
/// spoken for has no PCI as far as this kernel is concerned, and everything
/// built so far works without it.
///
/// The walk **recurses through bridges** — QEMU's q35 puts a DMI-to-PCI bridge
/// at `00:1e.0` and VMware places functions behind `pciBridge0`–`pciBridge7`,
/// so a scan of bus zero can miss a controller entirely (`012` §2, D-168).
fn survey_the_bus() {
    if !wost::arch::current::pci::init() {
        warn!("pci: the configuration ports are already claimed; no bus");
        return;
    }

    let mut controller = None;
    let mut storage = 0;
    let surveyed = pci::enumerate(|function| {
        if function.class.is_nvme() {
            storage += 1;
            if controller.is_none() {
                controller = Some(function.at);
            }
        }
    });

    match surveyed {
        Some(survey) => info!(
            "pci: {} functions across {} bus(es), {} bridge(s), highest {:#04x}, {storage} nvme",
            survey.functions, survey.buses, survey.bridges, survey.highest_bus
        ),
        None => {
            warn!("pci: configuration space did not answer");
            return;
        }
    }

    let Some(at) = controller else {
        info!("pci: no nvme controller; nothing to claim");
        return;
    };
    claim_the_controller(at);
}

/// Claim the storage controller and say what it decodes.
///
/// The claim is what makes touching it legitimate, and it carries the windows
/// with it (D-168). Slice 5 is what does anything with them; this is the point
/// at which the shipped kernel and the checked one stop differing, because a
/// claim path nothing calls in the shipped build is a claim path only the
/// checks exercise.
fn claim_the_controller(at: pci_config::Address) {
    let mut device = match pci::claim(at) {
        Ok(device) => device,
        Err(reason) => {
            warn!(
                "nvme: {:02x}:{:02x}.{} could not be claimed: {reason}",
                at.bus(),
                at.device(),
                at.function()
            );
            return;
        }
    };

    let (vendor, model) = device.identity();
    info!(
        "nvme: claimed {:02x}:{:02x}.{} — {vendor:04x}:{model:04x}",
        at.bus(),
        at.device(),
        at.function()
    );

    // Bus master before anything else is attempted. Without it the controller
    // cannot DMA, queues never advance, and **there is no error anywhere to
    // read** (`012` §2).
    if device.enable_bus_master() {
        info!("nvme: memory space and bus master on");
    } else {
        warn!("nvme: the command register would not take bus master");
    }

    // BAR0 is the register file. A controller whose first window is not
    // memory is one this driver does not know how to reach, and saying so is
    // better than mapping whatever it is.
    let Some(window) = device.window(0) else {
        warn!("nvme: no bar0; nothing to map");
        return;
    };
    let pci_config::Bar::Memory { address, wide, .. } = window.bar else {
        warn!("nvme: bar0 is not a memory window");
        return;
    };
    info!(
        "nvme: bar0 is {} bytes of {}-bit memory at {address:#x}",
        window.size,
        if wide { 64 } else { 32 }
    );

    let mut context = wost::sync::ThreadContext::require();
    let Some(mut controller) = nvme::start(&mut context, address, window.size) else {
        return;
    };

    // One block, to prove the whole path: queue built, command submitted,
    // doorbell rung, phase tag recognised, completion matched by identifier.
    let Some(buffer) = wost::mm::allocate_contiguous(1, 1) else {
        warn!("nvme: no memory for a read buffer");
        return;
    };
    match controller.read_blocks(0, 1, &buffer) {
        Ok(()) => info!("nvme: read block 0"),
        Err(reason) => warn!("nvme: block 0 could not be read: {reason}"),
    }
    nvme::keep(controller, buffer);
    mount_the_volume();
}

/// Mount the volume and say what is on it, **through the VFS**.
///
/// Not through `fs::with_volume` directly, which would be the kernel reaching
/// past its own abstraction on the one path a person watches — and an
/// abstraction only the checks use is one the shipped kernel is not evidence
/// about (D-171).
///
/// Declines rather than fails: a namespace with no volume on it is a machine
/// with no files, and the kernel boots either way.
fn mount_the_volume() {
    let shape =
        fs::with_volume(|volume| Ok((volume.bpb().clusters(), volume.bpb().cluster_bytes())));
    match shape {
        None => {
            info!("fat32: no controller, so no volume");
            return;
        }
        Some(Err(reason)) => {
            warn!("fat32: the volume would not mount: {reason:?}");
            return;
        }
        Some(Ok((clusters, cluster_bytes))) => {
            info!("fat32: mounted — {clusters} clusters of {cluster_bytes} bytes");
        }
    }

    let Some(root) = vfs::root() else {
        return;
    };

    // Collected here and used after, because both operations reach the
    // controller through a leaf guard and resolving inside this callback would
    // re-enter it (D-171).
    let mut files = 0usize;
    let mut directories = 0usize;
    let mut first = [0u8; 16];
    let mut first_len = 0usize;
    if let Err(reason) = vfs::enumerate(root, |name, is_directory| {
        if is_directory {
            directories += 1;
        } else {
            files += 1;
            if first_len == 0 && name.len() <= first.len() {
                first[..name.len()].copy_from_slice(name);
                first_len = name.len();
            }
        }
        true
    }) {
        warn!("fat32: the root would not list: {reason:?}");
        return;
    }
    info!("fat32: {files} file(s) and {directories} directory(ies) in the root");

    if first_len == 0 {
        return;
    }
    let name = &first[..first_len];
    match vfs::resolve(root, name) {
        Ok(vfs::Node::File(file)) => {
            let mut head = [0u8; 32];
            match vfs::read(file, 0, &mut head) {
                Ok(read) => info!(
                    "fat32: {} — read {read} of its bytes",
                    core::str::from_utf8(name).unwrap_or("?")
                ),
                Err(reason) => warn!("fat32: that file would not read: {reason:?}"),
            }
        }
        Ok(vfs::Node::Directory(_)) => warn!("fat32: a listed file resolved to a directory"),
        Err(reason) => warn!("fat32: a listed name would not resolve: {reason:?}"),
    }
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
