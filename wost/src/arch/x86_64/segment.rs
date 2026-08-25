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

use super::paging::MappedStack;
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

/// Ring 3 data. Loaded into `SS` by the `iretq` that enters user code.
///
/// Placed **before** the user code segment, which looks backwards and is what
/// `sysret` requires: it derives `CS` from `STAR` + 16 and `SS` from
/// `STAR` + 8, so the data descriptor has to come first. Nothing uses `sysret`
/// yet and the order is fixed now anyway, because moving a descriptor later
/// means moving every selector that names it (D-96).
pub const USER_DATA: Selector = Selector::new(5, 3);

/// Ring 3 code.
pub const USER_CODE: Selector = Selector::new(6, 3);

/// [`USER_DATA`]'s bits, for the `iretq` frame that enters ring 3.
///
/// A `const` operand in inline assembly must be an integer, and `Selector` is
/// not one — so the value is named here rather than open-coded where it is
/// pushed.
pub(super) const USER_DATA_BITS: u16 = USER_DATA.bits();

/// [`KERNEL_DATA`]'s bits, for the `verw` operand on the return path.
pub(super) const KERNEL_DATA_BITS: u16 = KERNEL_DATA.bits();

/// [`USER_CODE`]'s bits, for the same reason.
pub(super) const USER_CODE_BITS: u16 = USER_CODE.bits();

/// Interrupt stack table entry holding the double-fault stack.
pub const DOUBLE_FAULT_STACK: u8 = 1;

/// Null, code, data, two for the task-state segment, then the user pair.
const ENTRIES: usize = 7;

/// Stack reserved for the double-fault handler. Generous because the fault it
/// handles is usually "the last stack ran out".
const FAULT_STACK_BYTES: usize = 16 * 1024;

/// Sixteen-byte aligned so the stack pointer handed to the CPU is too.
#[repr(C, align(16))]
struct FaultStack([u8; FAULT_STACK_BYTES]);

static GDT: HardwareCell<[u64; ENTRIES]> = HardwareCell::new([0; ENTRIES]);
/// The task-state segment, placed so its interrupt-stack entries are
/// eight-byte aligned.
///
/// The architecture puts `IST1` at offset `0x24`, and `TaskState` is
/// `packed(4)` because the first stack pointer is at offset 4. Together those
/// mean that in a segment on an eight-byte boundary, `IST1` sits at four *past*
/// one — so an eight-byte store to it is misaligned, which is undefined
/// behaviour however well x86 tolerates it in practice.
///
/// Four bytes of padding in front moves the whole segment to four-mod-eight,
/// which puts `0x24` back on a boundary. That is what makes
/// [`set_double_fault_stack`]'s store both sound and atomic, and a host test
/// in `x86-tables` pins the offset it depends on (D-95).
#[repr(C, align(8))]
struct AlignedTaskState {
    /// Never read. Its only job is to shift `state` off the boundary.
    _leading: u32,
    state: TaskState,
}

static TASK_STATE_SEGMENT: HardwareCell<AlignedTaskState> = HardwareCell::new(AlignedTaskState {
    _leading: 0,
    state: TaskState::new(),
});

/// The task-state segment itself, past the padding.
fn task_state_pointer() -> *mut TaskState {
    // SAFETY: `AlignedTaskState` is `repr(C)`, so this projects to a field of
    // the live static rather than deriving a pointer from an integer.
    unsafe { &raw mut (*TASK_STATE_SEGMENT.as_mut_ptr()).state }
}
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
    unsafe { task_state_pointer().write(task_state) };

    let task_state_descriptor =
        SystemDescriptor::task_state(task_state_pointer() as u64, size_of::<TaskState>() as u16);

    let mut entries = [0u64; ENTRIES];
    entries[0] = Descriptor::NULL.bits();
    entries[KERNEL_CODE.index() as usize] = Descriptor::KERNEL_CODE.bits();
    entries[KERNEL_DATA.index() as usize] = Descriptor::KERNEL_DATA.bits();
    entries[TASK_STATE.index() as usize] = task_state_descriptor.low;
    entries[TASK_STATE.index() as usize + 1] = task_state_descriptor.high;
    entries[USER_DATA.index() as usize] = Descriptor::USER_DATA.bits();
    entries[USER_CODE.index() as usize] = Descriptor::USER_CODE.bits();
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

/// Point `RSP0` at `top`, with no evidence asked for.
///
/// `pub(super)` and no further: the only caller is the context switch, and the
/// proof it stands on is one this module cannot express in a type — `task::init`
/// walked every stack in the arena and confirmed it mapped, guarded, writable
/// and never executable before it would fabricate on one (D-52), and these are
/// those stacks. A bare address from *outside* the arch module is exactly what
/// D-95 refused; from inside it, with that walk behind it, it is the same
/// evidence by a different route (D-99).
///
/// **There was a `pub fn set_kernel_stack(&MappedStack)` beside this, and it
/// is gone.** It asked for the evidence this one does not, and had no callers:
/// the switch writes `RSP0` *before* it installs the arriving thread's tables,
/// so `MappedStack::is_installed` — which walks the running tables since
/// D-125 — would refuse the arriving stack every time. A safe wrapper its only
/// intended caller cannot use by construction is the speculative API `005`
/// forbids, and keeping it suggested a road that was never open (D-129).
pub(super) fn write_kernel_stack(top: u64) {
    let at = TaskState::privilege_stack_offset(0);
    debug_assert!(
        task_state_pointer()
            .addr()
            .wrapping_add(at)
            .is_multiple_of(8),
        "the privilege-stack entry is not eight-byte aligned"
    );
    // SAFETY: `at` is a byte offset inside the segment, from the crate that
    // owns its layout, and the entry there is eight bytes on an eight-byte
    // boundary — `RSP0` is at offset 4, and the segment is placed
    // four-mod-eight so that it lands on one (D-95).
    let entry = unsafe { task_state_pointer().cast::<u8>().add(at) }.cast::<u64>();
    // SAFETY: as above; a naturally aligned eight-byte store, so a trap
    // arriving mid-switch reads one stack or the other and never a mixture.
    unsafe { entry.write_volatile(top) };
}

/// Where a trap from ring 3 will land.
pub fn kernel_stack() -> u64 {
    read_stack(TaskState::privilege_stack_offset(0))
}

/// Move the double-fault handler onto a guarded stack.
///
/// The stack `init` installs is a `.bss` array with nothing below it, which
/// `002`'s security baseline records as owed: a double fault that overruns it
/// runs into whatever the linker put next, silently. Once the tables are live a
/// guarded region can be mapped and the handler pointed at it (D-89).
///
/// Safe, and the reason is the width. The interrupt-stack table entry is a
/// single naturally-aligned eight-byte field, so this store is atomic on
/// x86-64: a double fault arriving during it sees either the old stack or the
/// new one, and both are mapped and writable. The old one is never unmapped —
/// it is in the kernel image — so there is no window in which the losing
/// value is invalid.
///
/// `false` if the stack does not belong to the address space this CPU is
/// walking, which would point the handler at memory it cannot reach.
pub fn set_double_fault_stack(stack: &MappedStack) -> bool {
    if !stack.is_installed() {
        return false;
    }
    // SAFETY: `MappedStack` is evidence that this region was mapped writable
    // with its guard verified absent, `is_installed` has just confirmed the
    // CPU walks the space that mapped it, and the field written is the
    // interrupt-stack entry alone — eight bytes, naturally aligned, so a
    // double fault arriving mid-store reads the old stack or the new one and
    // never a mixture. Both are mapped: the `.bss` array `init` installed is
    // in the kernel image and is never unmapped.
    let at = TaskState::interrupt_stack_offset(DOUBLE_FAULT_STACK as usize);
    // The alignment this store needs is arranged by `AlignedTaskState`, not
    // assumed: `IST1` is at `0x24`, which is four-mod-eight, so the segment is
    // deliberately placed four-mod-eight to compensate. Asserted rather than
    // trusted, because the two facts live in different crates.
    debug_assert!(
        task_state_pointer()
            .addr()
            .wrapping_add(at)
            .is_multiple_of(8),
        "the interrupt-stack entry is not eight-byte aligned"
    );
    // SAFETY: `at` is a byte offset inside the segment, from the crate that
    // owns its layout and checks it against the manual, and the assertion
    // above establishes the alignment this cast promises.
    let entry = unsafe { task_state_pointer().cast::<u8>().add(at) }.cast::<u64>();
    // SAFETY: as above; a naturally aligned eight-byte store, so the CPU reads
    // one value or the other and never a mixture.
    unsafe { entry.write_volatile(stack.top()) };
    true
}

/// Where the double-fault handler's stack currently starts.
pub fn double_fault_stack() -> u64 {
    read_stack(TaskState::interrupt_stack_offset(
        DOUBLE_FAULT_STACK as usize,
    ))
}

/// One stack pointer out of the segment, read the way it is written.
///
/// A single eight-byte load rather than a copy of the whole structure. The
/// stores are careful to be atomic against a trap arriving mid-write (D-95),
/// and a bulk `read_volatile` of a 104-byte structure undid that on the far
/// side: `RSP0` moves on every context switch, so a preempted reader could
/// see half of one stack and half of another (D-105).
fn read_stack(at: usize) -> u64 {
    debug_assert!(
        task_state_pointer()
            .addr()
            .wrapping_add(at)
            .is_multiple_of(8),
        "a stack pointer in the segment is not eight-byte aligned"
    );
    // SAFETY: `at` is a byte offset inside the segment, from the crate that
    // owns its layout, and the entry there is eight bytes on an eight-byte
    // boundary — the segment is placed four-mod-eight so that it is (D-95).
    let entry = unsafe { task_state_pointer().cast::<u8>().add(at) }.cast::<u64>();
    // SAFETY: as above; a naturally aligned eight-byte load, so it reads one
    // value that was written there and never a mixture.
    unsafe { entry.read_volatile() }
}

/// The address the double-fault stack grows down from.
fn fault_stack_top() -> u64 {
    FAULT_STACK.address() + FAULT_STACK_BYTES as u64
}
