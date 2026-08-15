//! The global descriptor table and the task-state segment.
//!
//! The bootloader leaves the kernel running on a descriptor table it owns and
//! is entitled to reuse, so [`init`] installs ours before anything else touches
//! the CPU. In 64-bit mode the table carries almost no information — the
//! interesting part is the task-state segment it points at, which is where a
//! gate finds the stack for a fault that cannot trust the current one.

use core::arch::asm;

use x86_tables::Selector;
use x86_tables::gdt::{Descriptor, SystemDescriptor};
use x86_tables::tss::TaskState;

use super::table::TablePointer;
use crate::cell::HardwareCell;
use crate::sync::Once;

/// Ring 0 code.
pub const KERNEL_CODE: Selector = Selector::new(1, 0);

/// Ring 0 data. `SS` holds this; `iretq` reloads it on every return.
pub const KERNEL_DATA: Selector = Selector::new(2, 0);

/// The task-state segment. Being a system descriptor it is sixteen bytes, so it
/// occupies this entry and the one after it.
pub const TASK_STATE: Selector = Selector::new(3, 0);

/// Interrupt stack table entry holding the double-fault stack.
pub const DOUBLE_FAULT_STACK: u8 = 1;

/// Null, code, data, and two for the task-state segment.
const ENTRIES: usize = 5;

/// Stack reserved for the double-fault handler. Generous because the fault it
/// handles is usually "the last stack ran out".
const FAULT_STACK_BYTES: usize = 16 * 1024;

/// Sixteen-byte aligned so the stack pointer handed to the CPU is too.
#[repr(C, align(16))]
struct FaultStack([u8; FAULT_STACK_BYTES]);

static GDT: HardwareCell<[u64; ENTRIES]> = HardwareCell::new([0; ENTRIES]);
static TASK_STATE_SEGMENT: HardwareCell<TaskState> = HardwareCell::new(TaskState::new());
static FAULT_STACK: HardwareCell<FaultStack> =
    HardwareCell::new(FaultStack([0; FAULT_STACK_BYTES]));
static INIT: Once = Once::new();

/// Install and load the kernel's descriptor table and task-state segment.
///
/// Reloads `CS`, `SS`, `DS` and `ES` from the new table and points the task
/// register at the new segment. `FS` and `GS` are left alone: nothing uses them
/// until per-CPU data arrives, and loading one would discard a base the
/// bootloader may still be relying on.
///
/// Does nothing after the first call.
pub fn init() {
    if !INIT.claim() {
        return;
    }

    let mut task_state = TaskState::new();
    task_state.set_interrupt_stack(DOUBLE_FAULT_STACK as usize, fault_stack_top());
    // SAFETY: `INIT` admitted this caller alone, and nothing reads the segment
    // until the `ltr` below, so no reference to it can exist yet.
    unsafe { TASK_STATE_SEGMENT.as_mut_ptr().write(task_state) };

    let task_state_descriptor =
        SystemDescriptor::task_state(TASK_STATE_SEGMENT.address(), size_of::<TaskState>() as u16);

    let mut entries = [0u64; ENTRIES];
    entries[0] = Descriptor::NULL.bits();
    entries[KERNEL_CODE.index() as usize] = Descriptor::KERNEL_CODE.bits();
    entries[KERNEL_DATA.index() as usize] = Descriptor::KERNEL_DATA.bits();
    entries[TASK_STATE.index() as usize] = task_state_descriptor.low;
    entries[TASK_STATE.index() as usize + 1] = task_state_descriptor.high;
    // SAFETY: as above — one writer, and the CPU is not pointed at the table
    // until the `lgdt` below.
    unsafe { GDT.as_mut_ptr().write(entries) };

    let pointer = TablePointer::new(GDT.address(), (size_of::<[u64; ENTRIES]>() - 1) as u16);
    // SAFETY: `lgdt` reads ten bytes at the operand and points the CPU at the
    // table they describe. Both are ours, correctly encoded and permanently
    // resident, and the entry the current `CS` will be reloaded from is present.
    unsafe {
        asm!(
            "lgdt [{pointer}]",
            pointer = in(reg) &raw const pointer,
            options(readonly, nostack, preserves_flags),
        );
    }

    reload_selectors();

    // SAFETY: `ltr` marks the descriptor busy and points the task register at
    // it. Entry `TASK_STATE` was just written as an available 64-bit
    // task-state segment, and nothing has loaded it before — `INIT` guarantees
    // this runs once, and loading a busy segment is the only way this faults.
    unsafe {
        asm!(
            "ltr {selector:x}",
            selector = in(reg) TASK_STATE.bits(),
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// The selector this CPU is executing under.
pub fn code_selector() -> Selector {
    let bits: u16;
    // SAFETY: reading a segment register has no preconditions and no effects.
    unsafe {
        asm!(
            "mov {selector:x}, cs",
            selector = out(reg) bits,
            options(nomem, nostack, preserves_flags),
        );
    }
    Selector::from_bits(bits)
}

/// The selector in the task register, or a null selector if none is loaded.
pub fn task_register() -> Selector {
    let bits: u16;
    // SAFETY: `str` reads the task register. It is unprivileged, has no effects,
    // and stores zero when no task-state segment has been loaded.
    unsafe {
        asm!(
            "str {selector:x}",
            selector = out(reg) bits,
            options(nomem, nostack, preserves_flags),
        );
    }
    Selector::from_bits(bits)
}

/// Load the segment registers from the table [`init`] just installed.
fn reload_selectors() {
    // SAFETY: a far return pops an instruction pointer and a code selector,
    // both of which this sequence pushes: the address is a label inside the
    // block, and the selector is the present, executable, ring 0 entry written
    // above. `nostack` is deliberately absent — the stack is the mechanism.
    unsafe {
        asm!(
            "push {selector}",
            "lea {resume}, [rip + 2f]",
            "push {resume}",
            "retfq",
            "2:",
            selector = in(reg) u64::from(KERNEL_CODE.bits()),
            resume = out(reg) _,
            options(preserves_flags),
        );
    }

    // SAFETY: all three registers are loaded with a present, writable ring 0
    // data selector. `DS` and `ES` are ignored in 64-bit mode; `SS` is not, and
    // must name a valid descriptor before the first `iretq`.
    unsafe {
        asm!(
            "mov ss, {selector:x}",
            "mov ds, {selector:x}",
            "mov es, {selector:x}",
            selector = in(reg) KERNEL_DATA.bits(),
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// The address the double-fault stack grows down from.
fn fault_stack_top() -> u64 {
    FAULT_STACK.address() + FAULT_STACK_BYTES as u64
}
