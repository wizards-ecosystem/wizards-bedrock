use core::arch::asm;
use core::mem::offset_of;

use x86_tables::Selector;
use x86_tables::gdt::{Descriptor, SystemDescriptor};
use x86_tables::tss::TaskState;

use super::kpti::TransitionStackTop;
use super::percpu;
use super::table::TablePointer;
use super::task::KernelStackTop;
use crate::cell::HardwareCell;
use crate::sync::Once;

pub(crate) const KERNEL_CODE: Selector = Selector::new(1, 0);

const KERNEL_DATA: Selector = Selector::new(2, 0);

const TASK_STATE: Selector = Selector::new(3, 0);

pub(crate) const USER_DATA: Selector = Selector::new(5, 3);

pub(crate) const USER_CODE: Selector = Selector::new(6, 3);

pub(super) const USER_DATA_BITS: u16 = USER_DATA.bits();

pub(super) const KERNEL_DATA_BITS: u16 = KERNEL_DATA.bits();

pub(super) const USER_CODE_BITS: u16 = USER_CODE.bits();

pub(crate) const DOUBLE_FAULT_STACK: u8 = 1;

pub(crate) const NON_MASKABLE_STACK: u8 = 2;

pub(crate) const MACHINE_CHECK_STACK: u8 = 3;

const FAULT_STACKS: [u8; 3] = [DOUBLE_FAULT_STACK, NON_MASKABLE_STACK, MACHINE_CHECK_STACK];

const KERNEL_STACK_ENTRY: usize = TaskState::privilege_stack_offset(0);

const ENTRIES: usize = 7;

const BOOT_STACK_BYTES: usize = 16 * 1024;

// IST1 sits at 0x24 of the `packed(4)` segment; the four bytes after the GDT align it to eight.
#[derive(Debug)]
#[repr(C, align(8))]
pub(super) struct Tables {
    gdt: [u64; ENTRIES],
    _leading: u32,
    state: TaskState,
}

const _: () = assert!(
    offset_of!(Tables, state) % 8 == 4,
    "the task state must start four bytes past an eight-byte boundary"
);

#[derive(Debug)]
#[repr(C, align(16))]
struct BootStack([u8; BOOT_STACK_BYTES]);

static BOOT_TABLES: HardwareCell<Tables> = HardwareCell::new(Tables {
    gdt: [0; ENTRIES],
    _leading: 0,
    state: TaskState::new(),
});

// Until its entry area exists the boot processor's three fault vectors share this stack; none returns.
static BOOT_FAULT_STACK: HardwareCell<BootStack> =
    HardwareCell::new(BootStack([0; BOOT_STACK_BYTES]));

static INIT: Once = Once::new();

pub(super) fn boot_tables() -> *mut Tables {
    BOOT_TABLES.as_mut_ptr()
}

pub fn init() {
    if !INIT.claim() {
        return;
    }
    let top = BOOT_FAULT_STACK.address() + BOOT_STACK_BYTES as u64;
    // SAFETY: `INIT` admitted this caller alone, so no processor has loaded the static tables and
    // nothing else references them, and the boot fault stack serves only the fault vectors.
    unsafe { install(BOOT_TABLES.as_mut_ptr(), [top; 3]) };
}

fn task_state_in(tables: *mut Tables) -> *mut TaskState {
    tables
        .cast::<u8>()
        .wrapping_add(offset_of!(Tables, state))
        .cast()
}

/// # Safety
///
/// `tables` must be writable for as long as the processor runs, referenced by nothing else and
/// loaded by no processor, and each top must end a permanently mapped stack only its vector uses.
pub(super) unsafe fn install(tables: *mut Tables, fault_stacks: [u64; 3]) {
    let state_address = task_state_in(tables);
    let mut task_state = TaskState::new();
    for (index, top) in FAULT_STACKS.into_iter().zip(fault_stacks) {
        task_state.set_interrupt_stack(usize::from(index), top);
    }
    // SAFETY: the caller hands over `tables` unreferenced and unloaded, so nothing reads the
    // segment until the `ltr` below.
    unsafe { core::ptr::write(state_address, task_state) };

    let task_state_descriptor =
        SystemDescriptor::task_state(state_address as u64, size_of::<TaskState>() as u16);

    let mut entries = [0u64; ENTRIES];
    entries[0] = Descriptor::NULL.bits();
    entries[KERNEL_CODE.index() as usize] = Descriptor::KERNEL_CODE.bits();
    entries[KERNEL_DATA.index() as usize] = Descriptor::KERNEL_DATA.bits();
    entries[TASK_STATE.index() as usize] = task_state_descriptor.low;
    entries[TASK_STATE.index() as usize + 1] = task_state_descriptor.high;
    entries[USER_DATA.index() as usize] = Descriptor::USER_DATA.bits();
    entries[USER_CODE.index() as usize] = Descriptor::USER_CODE.bits();
    let gdt = tables.cast::<[u64; ENTRIES]>();
    // SAFETY: as above: one writer, and the CPU is not pointed at the table until the `lgdt` below.
    unsafe { core::ptr::write(gdt, entries) };

    let pointer = TablePointer::new(gdt as u64, (size_of::<[u64; ENTRIES]>() - 1) as u16);
    // SAFETY: `lgdt` loads the table its ten-byte operand describes; it is correctly encoded and
    // resident for good, and it holds the selectors this processor reloads next.
    unsafe {
        asm!(
            "lgdt [{pointer}]",
            pointer = in(reg) &raw const pointer,
            options(readonly, nostack, preserves_flags),
        );
    }

    reload_selectors();

    // SAFETY: `ltr` loads `TASK_STATE`, just written as a present, available 64-bit task-state
    // segment, and marks it busy; no processor loaded these tables before, so it is not busy.
    unsafe {
        asm!(
            "ltr {selector:x}",
            selector = in(reg) TASK_STATE.bits(),
            options(nostack, preserves_flags),
        );
    }
}

pub(super) fn current_code_selector() -> Selector {
    let code: u16;
    // SAFETY: reading a segment register has no preconditions and no effects.
    unsafe {
        asm!(
            "mov {code:x}, cs",
            code = out(reg) code,
            options(nomem, nostack, preserves_flags),
        );
    }
    Selector::from_bits(code)
}

// `FS` and `GS` keep their selectors; their bases are set by `wrmsr`.
fn reload_selectors() {
    // SAFETY: the far return pops the in-block label and the present, executable ring 0 selector
    // this sequence pushes; `nostack` is deliberately absent, since the stack is the mechanism.
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

    // SAFETY: all three get a present, writable ring 0 data selector; 64-bit mode ignores `DS` and
    // `ES` but not `SS`, which must name a valid descriptor before the first `iretq`.
    unsafe {
        asm!(
            "mov ss, {selector:x}",
            "mov ds, {selector:x}",
            "mov es, {selector:x}",
            selector = in(reg) KERNEL_DATA.bits(),
            options(nostack, preserves_flags),
        );
    }
}

// A selector one thread loaded must not reach the next; nulling `FS` may zero its base.
pub(super) fn reset_data_selectors() {
    assert!(
        !super::interrupt::are_enabled(),
        "data selectors reset only with interrupts excluded"
    );
    // SAFETY: `DS` and `ES` get the present ring-3 data descriptor, which ring 0 may load and
    // 64-bit mode ignores; a null `FS` is always legal, and the caller rewrites its base.
    unsafe {
        asm!(
            "mov ds, {data:x}",
            "mov es, {data:x}",
            "mov fs, {null:x}",
            data = in(reg) USER_DATA.bits(),
            null = in(reg) 0u16,
            options(nostack, preserves_flags),
        );
    }
    // SAFETY: the null load may zero only the base swapped under `GS`, which is ring 3's and
    // already zero, and the second `swapgs` restores this processor's entry area; nothing between
    // can fault, and the paranoid vectors read `IA32_GS_BASE` instead of trusting either state.
    unsafe {
        asm!(
            "swapgs",
            "mov gs, {null:x}",
            "swapgs",
            null = in(reg) 0u16,
            options(nostack, preserves_flags),
        );
    }
}

pub(super) fn write_kernel_stack(top: KernelStackTop<'_>) {
    write_stack(KERNEL_STACK_ENTRY, top.address());
}

pub(super) fn write_transition_stack(top: TransitionStackTop) {
    assert!(
        !super::interrupt::are_enabled(),
        "entry stacks change only with interrupts excluded"
    );
    write_stack(KERNEL_STACK_ENTRY, top.address());
}

fn write_stack(offset: usize, top: u64) {
    let entry = stack_entry(offset);
    // SAFETY: `stack_entry` asserted the field eight-byte aligned at a layout-owned offset inside
    // this processor's live segment, so an arriving trap observes one complete stack or the other.
    unsafe { entry.write_volatile(top) };
}

fn stack_entry(offset: usize) -> *mut u64 {
    let state = task_state_in(percpu::entry().tables());
    assert!(
        state.addr().wrapping_add(offset).is_multiple_of(8),
        "a stack pointer in the segment is not eight-byte aligned"
    );
    state.cast::<u8>().wrapping_add(offset).cast::<u64>()
}

pub(super) fn kernel_stack() -> u64 {
    read_stack(KERNEL_STACK_ENTRY)
}

// One eight-byte load, as written: a bulk copy could see half of one stack and half of another.
fn read_stack(offset: usize) -> u64 {
    let entry = stack_entry(offset);
    // SAFETY: `stack_entry` asserted the field eight-byte aligned at a layout-owned offset inside
    // the live segment, so the load observes one complete value written there.
    unsafe { entry.read_volatile() }
}
