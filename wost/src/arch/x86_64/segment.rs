use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use x86_tables::Selector;
use x86_tables::gdt::{Descriptor, SystemDescriptor};
use x86_tables::tss::TaskState;

use super::paging::{KernelStackTop, PinnedStack};
use super::table::TablePointer;
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

const DOUBLE_FAULT_ENTRY: usize = TaskState::interrupt_stack_offset(DOUBLE_FAULT_STACK as usize);

const KERNEL_STACK_ENTRY: usize = TaskState::privilege_stack_offset(0);

const ENTRIES: usize = 7;

const FAULT_STACK_BYTES: usize = 16 * 1024;

#[derive(Debug)]
#[repr(C, align(16))]
struct FaultStack([u8; FAULT_STACK_BYTES]);

#[unsafe(link_section = ".kpti.tables.gdt")]
static GDT: HardwareCell<[u64; ENTRIES]> = HardwareCell::new([0; ENTRIES]);

// IST1 sits at 0x24 of the `packed(4)` segment; four leading bytes put it on an 8-byte boundary.
#[derive(Debug)]
#[repr(C, align(8))]
struct AlignedTaskState {
    _leading: u32,
    state: TaskState,
}

#[unsafe(link_section = ".kpti.tables.tss")]
static TASK_STATE_SEGMENT: HardwareCell<AlignedTaskState> = HardwareCell::new(AlignedTaskState {
    _leading: 0,
    state: TaskState::new(),
});

fn task_state_pointer() -> *mut TaskState {
    // SAFETY: `AlignedTaskState` is `repr(C)`, so this projects to a field of the live static
    // instead of deriving a pointer from an integer.
    unsafe { &raw mut (*TASK_STATE_SEGMENT.as_mut_ptr()).state }
}

#[unsafe(link_section = ".kpti.state.ist_stack")]
static FAULT_STACK: HardwareCell<FaultStack> =
    HardwareCell::new(FaultStack([0; FAULT_STACK_BYTES]));
static ORDINARY_FAULT_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static INIT: Once = Once::new();

pub fn init() {
    if !INIT.claim() {
        return;
    }

    let mut task_state = TaskState::new();
    task_state.set_interrupt_stack(DOUBLE_FAULT_STACK as usize, fault_stack_top());
    // SAFETY: `INIT` admitted this caller alone, and nothing reads the segment until the `ltr`
    // below, so no reference to it can exist yet.
    unsafe { core::ptr::write(task_state_pointer(), task_state) };

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
    // SAFETY: as above: one writer, and the CPU is not pointed at the table until the `lgdt` below.
    unsafe { core::ptr::write(GDT.as_mut_ptr(), entries) };

    let pointer = TablePointer::new(GDT.address(), (size_of::<[u64; ENTRIES]>() - 1) as u16);
    // SAFETY: `lgdt` loads the table its ten-byte operand describes; both are ours, correctly
    // encoded and permanently resident, and the entry `CS` will be reloaded from is present.
    unsafe {
        asm!(
            "lgdt [{pointer}]",
            pointer = in(reg) &raw const pointer,
            options(readonly, nostack, preserves_flags),
        );
    }

    reload_selectors();

    // SAFETY: `ltr` loads `TASK_STATE`, just written as a present, available 64-bit task-state
    // segment, and marks it busy; `INIT` runs this once, so it cannot already be busy.
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

// `FS` and `GS` keep the bootloader's selectors and bases.
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
    // SAFETY: `DS` and `ES` get the present ring-3 data descriptor, which ring 0 may load and
    // 64-bit mode ignores; a null `FS` or `GS` is always legal, and the caller rewrites the base.
    unsafe {
        asm!(
            "mov ds, {data:x}",
            "mov es, {data:x}",
            "mov fs, {null:x}",
            "mov gs, {null:x}",
            data = in(reg) USER_DATA.bits(),
            null = in(reg) 0u16,
            options(nostack, preserves_flags),
        );
    }
}

pub(super) fn write_kernel_stack(top: KernelStackTop<'_>) {
    write_stack(KERNEL_STACK_ENTRY, top.address());
}

pub(super) fn install_entry_stacks(
    transition: super::kpti::TransitionStackTop,
    interrupt: IstStackTop,
) {
    assert!(
        !super::interrupt::are_enabled(),
        "entry stacks change only with interrupts excluded"
    );
    write_stack(KERNEL_STACK_ENTRY, transition.address());
    write_stack(DOUBLE_FAULT_ENTRY, interrupt.address());
}

pub(super) fn restore_ordinary_fault_stack() {
    assert!(
        !super::interrupt::are_enabled(),
        "the interrupt stack changes only with interrupts excluded"
    );
    let top = ORDINARY_FAULT_STACK_TOP.load(Ordering::Acquire);
    assert!(
        top != 0,
        "the guarded ordinary fault stack is not installed"
    );
    write_stack(DOUBLE_FAULT_ENTRY, top);
}

fn write_stack(offset: usize, top: u64) {
    let entry = stack_entry(offset);
    // SAFETY: `stack_entry` asserted the field eight-byte aligned at a layout-owned offset inside
    // the live segment, so an arriving trap observes one complete stack or the other.
    unsafe { entry.write_volatile(top) };
}

fn stack_entry(offset: usize) -> *mut u64 {
    assert!(
        task_state_pointer()
            .addr()
            .wrapping_add(offset)
            .is_multiple_of(8),
        "a stack pointer in the segment is not eight-byte aligned"
    );
    task_state_pointer()
        .cast::<u8>()
        .wrapping_add(offset)
        .cast::<u64>()
}

pub(super) fn kernel_stack() -> u64 {
    read_stack(KERNEL_STACK_ENTRY)
}

// Safe: `PinnedStack` cannot be revoked, and the stack it replaces is a kernel-image array.
#[must_use = "a refused double-fault stack install changed no TSS entry"]
pub fn set_double_fault_stack(stack: &PinnedStack) -> bool {
    if !stack.is_installed() || super::kpti::entry_is_ready() {
        return false;
    }
    let top = stack.top();
    write_stack(DOUBLE_FAULT_ENTRY, top);
    ORDINARY_FAULT_STACK_TOP.store(top, Ordering::Release);
    true
}

pub(super) fn double_fault_stack_address() -> u64 {
    read_stack(DOUBLE_FAULT_ENTRY)
}

// One eight-byte load, as written: a bulk copy could see half of one stack and half of another.
fn read_stack(offset: usize) -> u64 {
    let entry = stack_entry(offset);
    // SAFETY: `stack_entry` asserted the field eight-byte aligned at a layout-owned offset inside
    // the live segment, so the load observes one complete value written there.
    unsafe { entry.read_volatile() }
}

fn fault_stack_top() -> u64 {
    FAULT_STACK.address() + FAULT_STACK_BYTES as u64
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct IstStackTop(u64);

impl IstStackTop {
    pub(super) const fn address(self) -> u64 {
        self.0
    }
}

pub(super) fn enclave_ist_stack_top() -> IstStackTop {
    IstStackTop(fault_stack_top())
}
