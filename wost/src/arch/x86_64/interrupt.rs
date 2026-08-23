//! Interrupt and exception entry.
//!
//! Rust cannot write an interrupt handler: the calling convention that would
//! let it is a nightly feature, and this kernel is on stable (D-01). So entry is
//! assembly — one stub per vector, which normalises what the CPU pushed, and one
//! common tail, which saves the registers and calls [`Handler`].
//!
//! Every gate is an interrupt gate, so a handler starts with interrupts
//! disabled. See `docs/006-cpu-fundamentals.md` for the frame layout argument.
//!
//! The interrupt flag itself lives here rather than in [`cpu`],
//! because the reason it is safe to set is a property of this module: the table
//! is loaded, and every one of its gates is present.

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use x86_tables::exception;
use x86_tables::idt::{self, Gate};

use super::cpu;
use super::pic;
use super::segment;
use super::table::TablePointer;
use crate::cell::HardwareCell;
use crate::sync::Once;

/// `RFLAGS` bit 9: this CPU accepts maskable interrupts.
pub(crate) const FLAGS_INTERRUPT: u64 = 1 << 9;

/// Bytes each entry stub occupies.
///
/// The generator pads every stub to this, which is what lets the table be
/// indexed by vector instead of naming 256 handlers. [`init`] checks the
/// assumption against the symbols bracketing the generated table rather than
/// trusting it.
const STUB_BYTES: usize = 16;

/// The registers a handler is given, in the order the entry path leaves them on
/// the stack.
///
/// Field order is not a matter of taste: it mirrors the pushes in the entry
/// sequence exactly, lowest address first. The selftest checks the two still
/// agree on a live machine, because nothing else can.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TrapFrame {
    /// Saved `r15`.
    pub r15: u64,
    /// Saved `r14`.
    pub r14: u64,
    /// Saved `r13`.
    pub r13: u64,
    /// Saved `r12`.
    pub r12: u64,
    /// Saved `r11`.
    pub r11: u64,
    /// Saved `r10`.
    pub r10: u64,
    /// Saved `r9`.
    pub r9: u64,
    /// Saved `r8`.
    pub r8: u64,
    /// Saved `rbp`.
    pub rbp: u64,
    /// Saved `rdi`.
    pub rdi: u64,
    /// Saved `rsi`.
    pub rsi: u64,
    /// Saved `rdx`.
    pub rdx: u64,
    /// Saved `rcx`.
    pub rcx: u64,
    /// Saved `rbx`.
    pub rbx: u64,
    /// Saved `rax`.
    pub rax: u64,
    /// Which vector was raised.
    pub vector: u64,
    /// The error code the CPU pushed, or zero for a vector that pushes none.
    pub error_code: u64,
    /// Where execution was, and where `iretq` resumes.
    pub instruction_pointer: u64,
    /// The code selector at the point of the fault.
    pub code_selector: u64,
    /// The flags register at the point of the fault.
    pub flags: u64,
    /// The stack pointer at the point of the fault.
    pub stack_pointer: u64,
    /// The stack selector at the point of the fault.
    pub stack_selector: u64,
}

/// What the kernel does when a vector is raised.
///
/// Runs with interrupts disabled, on the interrupted stack unless the gate
/// selected one from the task-state segment. Returning resumes the interrupted
/// code, which is meaningful for a trap such as a breakpoint and is a loop for
/// a fault, whose saved instruction pointer is the instruction that faulted.
pub type Handler = fn(&mut TrapFrame);

static IDT: HardwareCell<[Gate; idt::TABLE_LEN]> =
    HardwareCell::new([Gate::MISSING; idt::TABLE_LEN]);
static HANDLER: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static INIT: Once = Once::new();

/// How many trap handlers are on this CPU's stack — zero in thread context.
///
/// What "in trap context" means, made a value something can read (D-55): the
/// heap refuses to allocate while it is non-zero, and the scheduler refuses
/// to yield. A context switch out of a handler parks the count with the
/// thread that owns it, so the thread switched *to* sees its own.
static TRAP_DEPTH: AtomicUsize = AtomicUsize::new(0);

/// Whether this CPU is inside a trap handler.
pub fn in_trap() -> bool {
    TRAP_DEPTH.load(Ordering::Relaxed) != 0
}

/// Take this context's trap depth off the CPU, for a switch to another.
///
/// Paired with [`resume_trap`] on the way back; between them the CPU runs a
/// thread whose depth is its own.
pub(crate) fn suspend_trap() -> usize {
    TRAP_DEPTH.swap(0, Ordering::Relaxed)
}

/// Put back the depth [`suspend_trap`] took.
pub(crate) fn resume_trap(depth: usize) {
    TRAP_DEPTH.store(depth, Ordering::Relaxed);
}

/// Fill the interrupt descriptor table and load it, routing every vector to
/// `handler`.
///
/// All 256 gates are filled. The architecture defines 32 of them; the rest
/// would otherwise turn a stray interrupt into a general protection fault, then
/// a double fault, then a reset with nothing printed.
///
/// Does nothing after the first call.
///
/// # Panics
///
/// Panics if the global descriptor table has not been installed — the gates
/// name its code selector — or if the generated stub table is not the size the
/// indexing assumes.
pub fn init(handler: Handler) {
    if !INIT.claim() {
        return;
    }

    assert!(
        segment::code_selector() == segment::KERNEL_CODE,
        "install the global descriptor table before the interrupt one"
    );

    let base = stub_table as *const () as usize;
    let span = stub_table_end as *const () as usize - base;
    assert!(
        span == idt::TABLE_LEN * STUB_BYTES,
        "entry stub table is the wrong size for a stub of STUB_BYTES"
    );

    HANDLER.store(handler as *const () as *mut (), Ordering::Release);

    // SAFETY: `INIT` admitted this caller alone and the table is reachable only
    // through this module, so no other reference to it exists. The CPU is not
    // pointed at it until the `lidt` below.
    let gates = unsafe { &mut *IDT.as_mut_ptr() };
    for (vector, gate) in gates.iter_mut().enumerate() {
        let stack = if vector == exception::DOUBLE_FAULT as usize {
            segment::DOUBLE_FAULT_STACK
        } else {
            0
        };
        *gate = Gate::interrupt(
            (base + vector * STUB_BYTES) as u64,
            segment::KERNEL_CODE,
            stack,
        );
    }

    let pointer = TablePointer::new(IDT.address(), idt::TABLE_LIMIT);
    // SAFETY: `lidt` reads ten bytes at the operand and points the CPU at the
    // table they describe. Every gate in it was just written present, with a
    // valid code selector and a handler address inside the stub table.
    unsafe {
        asm!(
            "lidt [{pointer}]",
            pointer = in(reg) &raw const pointer,
            options(readonly, nostack, preserves_flags),
        );
    }
}

/// Whether the CPU's interrupt descriptor table register points at the table
/// [`init`] built, with the limit it wrote.
pub fn table_is_loaded() -> bool {
    let mut pointer = TablePointer::empty();
    // SAFETY: `sidt` stores ten bytes at the operand, which is a `TablePointer`
    // this call owns.
    unsafe {
        asm!(
            "sidt [{pointer}]",
            pointer = in(reg) &raw mut pointer,
            options(nostack, preserves_flags),
        );
    }
    pointer.base() == IDT.address() && pointer.limit() == idt::TABLE_LIMIT
}

/// Whether this CPU was accepting interrupts, captured so that a critical
/// section can put the flag back the way it found it rather than guessing.
///
/// Restoring rather than enabling is what makes nesting work: an inner section
/// that saw the flag already clear puts back a clear flag, so the outer section
/// keeps the exclusion it was relying on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptState(bool);

impl InterruptState {
    /// Put the interrupt flag back the way it was.
    pub fn restore(self) {
        if self.0 {
            set_flag();
        }
    }
}

/// Whether this CPU is currently accepting maskable interrupts.
pub fn are_enabled() -> bool {
    flags() & FLAGS_INTERRUPT != 0
}

/// Stop accepting maskable interrupts, reporting what it took to do so.
///
/// The report is the argument for [`InterruptState::restore`]; dropping it
/// leaves interrupts off, which is always safe and occasionally what a caller
/// means.
pub fn disable() -> InterruptState {
    let previous = are_enabled();
    // SAFETY: `cli` clears IF on the current CPU and touches nothing else. It
    // has no preconditions in ring 0. `preserves_flags` is deliberately absent —
    // modifying IF is the whole point.
    unsafe { asm!("cli", options(nomem, nostack)) };
    InterruptState(previous)
}

/// Begin accepting maskable interrupts on this CPU.
///
/// Safe rather than `unsafe` because both of its preconditions are checkable
/// and are checked. That is the point: the version of this function deleted in
/// Phase 1 could only have stated them in prose.
///
/// # Panics
///
/// Panics unless the interrupt table is loaded and the legacy controller has
/// been silenced. With no table, the first interrupt is a reset with nothing
/// printed; with a live 8259 the firmware left pointing at the exception
/// vectors, the first timer tick arrives as a double fault.
pub fn enable() {
    assert!(
        table_is_loaded(),
        "load the interrupt table before accepting interrupts"
    );
    assert!(
        pic::is_silenced(),
        "silence the legacy controller before accepting interrupts"
    );
    set_flag();
}

/// Begin accepting interrupts and halt, as one step.
///
/// What a thread with nothing to do until a deadline does with the flag
/// clear. The two halves cannot be separate calls: an interrupt arriving
/// between them would be taken before the halt, and the CPU would then halt
/// with the thing that was going to wake it already spent. The architecture
/// closes exactly that window — `sti` does not admit an interrupt until after
/// the instruction following it, which here is the `hlt`.
///
/// Returns once an interrupt has been taken, with the flag left set.
///
/// # Panics
///
/// As [`enable`], whose preconditions this shares: with no table loaded or a
/// live legacy controller, the interrupt that ends the halt is a reset.
pub fn enable_and_halt() {
    assert!(
        table_is_loaded(),
        "load the interrupt table before accepting interrupts"
    );
    assert!(
        pic::is_silenced(),
        "silence the legacy controller before accepting interrupts"
    );
    // SAFETY: `sti` sets IF and `hlt` stops the CPU until an interrupt
    // arrives; both are unprivileged in ring 0 and neither touches memory.
    // The preconditions for setting the flag are the two asserts above, and
    // the pair cannot deadlock the CPU because the shadow makes the halt
    // reachable only with interrupts already admitted.
    unsafe { asm!("sti", "hlt", options(nomem, nostack, preserves_flags)) };
}

/// Set the interrupt flag with nothing checked.
///
/// Private, and the difference between it and [`enable`] is the whole design:
/// restoring a flag that was set is safe because setting it was, whereas
/// setting it the first time is the step that needs an argument.
fn set_flag() {
    // SAFETY: `sti` sets IF on the current CPU and touches nothing else. Every
    // caller either checked the preconditions or is restoring a flag whose
    // caller did.
    unsafe { asm!("sti", options(nomem, nostack)) };
}

/// The flags register.
fn flags() -> u64 {
    let value: u64;
    // SAFETY: `pushfq` pushes the flags register and `pop` takes the same eight
    // bytes back off, so the stack ends as it started. `nomem` is accurate:
    // nothing outside the asm can observe the slot in between.
    unsafe {
        asm!(
            "pushfq",
            "pop {value}",
            value = out(reg) value,
            options(nomem, preserves_flags),
        );
    }
    value
}

/// Raise a breakpoint exception on this CPU.
///
/// The trap a debugger plants, and the only exception that can be raised
/// deliberately and returned from: the saved instruction pointer is already
/// past the instruction that caused it.
pub fn breakpoint() {
    // SAFETY: `int3` raises vector 3, whose gate is present. No options are
    // given because the handler may do anything a Rust function may do.
    unsafe { asm!("int3") };
}

/// Call the registered handler with the frame the entry path saved.
extern "C" fn dispatch(frame: *mut TrapFrame) {
    let registered = HANDLER.load(Ordering::Acquire);
    if registered.is_null() {
        cpu::halt_forever();
    }

    // SAFETY: the slot is written only by `init`, from a `Handler`, and only
    // before the table naming this entry path is loaded. A function pointer and
    // a data pointer are the same width on x86_64.
    let handler: Handler = unsafe { core::mem::transmute(registered) };

    // SAFETY: the entry path passes the register block it has just pushed. It
    // outlives this call, and nothing else can hold a reference to it: this CPU
    // was interrupted, and it entered through a gate that cleared the interrupt
    // flag.
    let frame = unsafe { &mut *frame };

    TRAP_DEPTH.fetch_add(1, Ordering::Relaxed);
    handler(frame);
    TRAP_DEPTH.fetch_sub(1, Ordering::Relaxed);
}

unsafe extern "C" {
    /// First byte of the generated stub table. Declared as a function because
    /// that is what it is, and because the address of one can be taken in safe
    /// code.
    #[link_name = "wost_interrupt_stubs"]
    fn stub_table();

    /// One byte past the last stub.
    #[link_name = "wost_interrupt_stubs_end"]
    fn stub_table_end();
}

global_asm!(
    // One stub per vector, each padded to the same size so the table can be
    // indexed rather than named. Vectors the CPU pushes no error code for push
    // a zero, so every saved frame has one shape; `ERROR_CODE_VECTORS` is the
    // single source of truth for which those are, shared with the host tests
    // that check it against the SDM's table.
    ".balign {stub_bytes}",
    ".globl wost_interrupt_stubs",
    "wost_interrupt_stubs:",
    ".set wost_vector, 0",
    ".rept {architectural}",
    ".balign {stub_bytes}",
    ".if (({error_code_vectors} >> wost_vector) & 1) == 0",
    "push 0",
    ".endif",
    "push wost_vector",
    "jmp wost_interrupt_entry",
    ".set wost_vector, wost_vector + 1",
    ".endr",
    ".rept {interrupts}",
    ".balign {stub_bytes}",
    "push 0",
    "push wost_vector",
    "jmp wost_interrupt_entry",
    ".set wost_vector, wost_vector + 1",
    ".endr",
    ".balign {stub_bytes}",
    ".globl wost_interrupt_stubs_end",
    "wost_interrupt_stubs_end:",

    // The common tail. `cld` because the ABI requires a clear direction flag
    // and an interrupt does not clear it; `mov rdi, rsp` because the frame the
    // pushes just built is the dispatcher's argument. The stack is 16-byte
    // aligned at the call by construction: the CPU aligns to 16 before pushing
    // its five words, and the error code, the vector and fifteen registers put
    // it back. Floating-point state is not saved because the kernel is built
    // without SSE and cannot disturb it.
    "wost_interrupt_entry:",
    "cld",
    "push rax",
    "push rbx",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push rbp",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov rdi, rsp",
    "call {dispatch}",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rbp",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "pop rax",
    "add rsp, 16",
    "iretq",
    stub_bytes = const STUB_BYTES,
    architectural = const exception::COUNT,
    interrupts = const idt::TABLE_LEN - exception::COUNT as usize,
    error_code_vectors = const exception::ERROR_CODE_VECTORS,
    dispatch = sym dispatch,
);
