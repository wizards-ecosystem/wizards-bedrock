#![no_std]
#![no_main]
// Denied, since `forbid` would refuse the `#[allow(unsafe_code)]` glue `wost::entry!` expands here.
#![deny(unsafe_code)]

// Nothing may allocate before `mem::init` has run.
extern crate alloc;

mod acpi;
mod arch;
mod arena;
mod banner;
mod console;
mod dev;
mod devfs;
mod fs;
mod handle;
mod input;
mod interrupts;
mod iommu;
mod log;
mod mem;
mod nvme;
mod object;
mod panic;
mod pci;
mod queue;
mod random;
mod rank;
mod routing;
#[cfg(feature = "selftest")]
mod selftest;
mod storage;
mod theme;
mod thread;
mod time;
mod timepage;
mod tmpfs;
mod user;
mod vfs;
mod wait;
mod world;

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use boot_record::{Fact, Protections, Record};
use wost::arch::current::{cpu, segment, task, xstate};
use wost::boot::BootInfo;
use wost::sync::{Leaf, SpinLock};

use crate::arch::current::uart::UartError;
use crate::thread::ThreadContext;

::wost::entry!(kernel_main);

fn kernel_main(boot: BootInfo, mut context: ThreadContext) -> ! {
    let early_dma = iommu::secure_handoff(&boot);

    let serial = dev::init_uart();
    info!("The Wizard's Bedrock awakens.");

    match boot.framebuffer {
        Ok(framebuffer) => dev::init_console(framebuffer),
        Err(reason) => warn!("framebuffer: {reason}"),
    }

    // Checked only once both consoles are up, so either one shows why the boot stopped.
    let mut early_dma = match early_dma {
        Ok(boundary) => boundary,
        Err(reason) => panic!("DMA-safe boot handoff refused: {reason}"),
    };
    info!("limine: base revision supported");
    info!(
        "iommu: entry DMA fence covers {} PCI function(s), disabled {} inherited bus master(s), retained {} qualified non-requester(s)",
        early_dma.functions(),
        early_dma.disabled(),
        early_dma.retained_non_requesters()
    );
    match early_dma.retained_non_requesters() {
        0 => {
            info!("iommu: entry DMA fence policy strict");
            info!("{}", Record::Fact(Fact::DmaFenceStrict));
        }
        1 => info!(
            "iommu: entry DMA fence retained VMware 440BX 00:00.0 as the qualified non-requester"
        ),
        count => panic!("DMA-safe boot handoff classified {count} non-requesters"),
    }
    match early_dma.quiesced_requesters() {
        0 => {}
        1 => info!(
            "iommu: entry DMA fence quiesced VMware PIIX4 00:07.0 at its masked 8237 controllers; it remains a requester"
        ),
        count => panic!("DMA-safe boot handoff quiesced {count} ISA requesters"),
    }

    // The fault path exists before anything that can fault.
    segment::init();
    arch::current::trap::init();
    info!("cpu: descriptor tables installed");
    if let Err(reason) = cpu::establish_control_state() {
        warn!("cpu: {reason}");
    }
    let protections = cpu::enable_protections();
    info!("cpu: protections {protections}");
    info!(
        "{}",
        Record::Protections(Protections {
            write_protect: protections.write_protect,
            smep: protections.smep,
            smap: protections.smap,
            umip: protections.umip,
            nx: protections.nx,
        })
    );

    // Before any second thread exists, because every switch saves and restores from here on.
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
        None => {
            warn!("cpu: extended state unavailable; ring 3 stays closed");
            info!("{}", Record::Fact(Fact::ExtendedStateAbsent));
        }
    }

    let dma_non_requester = early_dma.take_non_requester();
    let dma_quiesced = early_dma.take_quiesced_requester();

    // The first write through an address the bootloader chose, so after the fault path.
    let layout = boot
        .memory
        .as_ref()
        .map(|memory| (*memory.map(), memory.direct()))
        .map_err(|reason| *reason);
    mem::init(boot.memory, early_dma);
    let interrupts = mem::init_paging(&mut context, layout, boot.image, boot.rsdp);
    if let Ok(rsdp) = boot.rsdp {
        iommu::discover(rsdp);
    }

    // The bootloader's stack sits in the unsplittable direct map: it cannot be guarded, only left.
    *HANDOFF.lock_leaf() = Some(Handoff {
        serial,
        interrupts,
        module: boot.module,
        dma_non_requester,
        dma_quiesced,
    });
    match mem::map_boot_stack(&mut context) {
        Some(stack) => task::continue_on_stack(context, stack, boot_continue, GUARDED_STACK),
        None => {
            warn!(
                "boot: staying on the bootloader's stack, which has no guard; ring 3 stays closed"
            );
            boot_continue(&mut context, BOOTLOADER_STACK)
        }
    }
}

// A static: a pointer into the abandoned bootloader stack would point at memory nothing owns.
#[derive(Debug)]
struct Handoff {
    serial: Result<(), UartError>,
    interrupts: Option<acpi::Interrupts>,
    module: Result<&'static [u8], wost::boot::HandoffError>,
    dma_non_requester: Option<pci_config::QualifiedNonRequester>,
    dma_quiesced: Option<pci_config::QuiescedIsaRequester>,
}

static HANDOFF: SpinLock<Leaf, Option<Handoff>> = SpinLock::new(None);

const BOOTLOADER_STACK: usize = 0;
const GUARDED_STACK: usize = 1;

static BOOT_STACK_GUARDED: AtomicBool = AtomicBool::new(false);

pub(crate) fn user_launch_is_safe() -> bool {
    BOOT_STACK_GUARDED.load(Ordering::Acquire)
}

fn boot_continue(context: &mut ThreadContext, stack: usize) -> ! {
    BOOT_STACK_GUARDED.store(stack == GUARDED_STACK, Ordering::Release);
    let Some(Handoff {
        serial,
        interrupts,
        module,
        dma_non_requester,
        dma_quiesced,
    }) = HANDOFF.lock_leaf().take()
    else {
        panic!("the boot handoff is gone; boot_continue ran twice");
    };

    dev::attach_back_buffer();
    let masked = routing::mask_external_controllers(context, interrupts);

    storage::survey_the_bus(context, dma_non_requester, dma_quiesced);
    storage::survey_the_filesystems();
    interrupts::start(masked.as_ref());
    match random::seed() {
        Ok(source) => info!("random: generator seeded with 256 bits credited from {source}"),
        Err(reason) => warn!("random: {reason}; ring 3 stays closed"),
    }
    world::start_root(context);
    input::init();
    routing::report_keyboard(masked);
    input::start_poller();
    queue::start_worker();

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

    // After interrupts are on, because a halting thread needs them to wake.
    thread::start_idle();

    #[cfg(feature = "selftest")]
    selftest::run(context);

    match user::run() {
        Ok(program) => info!("user: program running in slot {}", program.slot),
        Err(reason) => {
            warn!("user: no program: {reason}");
            if matches!(reason, user::SpawnError::NoInterruptAdmission) {
                info!("{}", Record::Fact(Fact::RingThreeClosedToInterrupts));
            }
        }
    }
    info!(
        "kernel: reading the keyboard, uptime {} ms",
        time::uptime_ms()
    );
    info!("{}", Record::Fact(Fact::Settled));
    echo_keystrokes(context)
}

// Drawn here, not by the keyboard producer, because a glyph can cost a scroll.
fn echo_keystrokes(context: &mut ThreadContext) -> ! {
    loop {
        let keystroke = input::read_key(context);
        info!("input: key consumed");
        info!("{}", Record::Fact(Fact::KeyConsumed));
        debug!("key: {:?} {:?}", keystroke.event.key, keystroke.event.state);
        if let Some(character) = keystroke.character {
            dev::with_console(|console| {
                let _ = console.write_char(character);
            });
        }
    }
}
