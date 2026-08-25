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
use core::fmt;
use core::marker::PhantomData;
use core::mem::size_of;
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

/// The registers the entry path leaves on the stack.
///
/// Field order is not a matter of taste: it mirrors the pushes in the entry
/// sequence exactly, lowest address first. The selftest checks the two still
/// agree on a live machine, because nothing else can.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct RawTrapFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rbp: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    vector: u64,
    error_code: u64,
    instruction_pointer: u64,
    code_selector: u64,
    flags: u64,
    stack_pointer: u64,
    stack_selector: u64,
}

const _: () = {
    assert!(core::mem::size_of::<RawTrapFrame>() == 22 * size_of::<u64>());
    assert!(core::mem::offset_of!(RawTrapFrame, r15) == 0);
    assert!(core::mem::offset_of!(RawTrapFrame, r8) == 7 * size_of::<u64>());
    assert!(core::mem::offset_of!(RawTrapFrame, rax) == 14 * size_of::<u64>());
    assert!(core::mem::offset_of!(RawTrapFrame, vector) == 15 * size_of::<u64>());
    assert!(core::mem::offset_of!(RawTrapFrame, flags) == 19 * size_of::<u64>());
    assert!(core::mem::offset_of!(RawTrapFrame, stack_selector) == 21 * size_of::<u64>());
};

/// A read-only snapshot of the registers saved on interrupt entry.
///
/// The assembly-owned frame is deliberately not exposed. A handler can inspect
/// where execution resumes, but safe code cannot redirect `iretq` or forge the
/// frame identity accepted by the scheduler.
pub struct TrapFrame {
    raw: RawTrapFrame,
    origin: *mut RawTrapFrame,
    stays_here: PhantomData<*mut ()>,
}

impl TrapFrame {
    /// Saved `r15`.
    pub const fn r15(&self) -> u64 {
        self.raw.r15
    }

    /// Saved `r14`.
    pub const fn r14(&self) -> u64 {
        self.raw.r14
    }

    /// Saved `r13`.
    pub const fn r13(&self) -> u64 {
        self.raw.r13
    }

    /// Saved `r12`.
    pub const fn r12(&self) -> u64 {
        self.raw.r12
    }

    /// Saved `r11`.
    pub const fn r11(&self) -> u64 {
        self.raw.r11
    }

    /// Saved `r10`.
    pub const fn r10(&self) -> u64 {
        self.raw.r10
    }

    /// Saved `r9`.
    pub const fn r9(&self) -> u64 {
        self.raw.r9
    }

    /// Saved `r8`.
    pub const fn r8(&self) -> u64 {
        self.raw.r8
    }

    /// Saved `rbp`.
    pub const fn rbp(&self) -> u64 {
        self.raw.rbp
    }

    /// Saved `rdi`.
    pub const fn rdi(&self) -> u64 {
        self.raw.rdi
    }

    /// Saved `rsi`.
    pub const fn rsi(&self) -> u64 {
        self.raw.rsi
    }

    /// Saved `rdx`.
    pub const fn rdx(&self) -> u64 {
        self.raw.rdx
    }

    /// Saved `rcx`.
    pub const fn rcx(&self) -> u64 {
        self.raw.rcx
    }

    /// Saved `rbx`.
    pub const fn rbx(&self) -> u64 {
        self.raw.rbx
    }

    /// Saved `rax`.
    pub const fn rax(&self) -> u64 {
        self.raw.rax
    }

    /// Which vector was raised.
    pub const fn vector(&self) -> u64 {
        self.raw.vector
    }

    /// The error code the CPU pushed, or zero for a vector that pushes none.
    pub const fn error_code(&self) -> u64 {
        self.raw.error_code
    }

    /// Where execution was, and where `iretq` resumes.
    pub const fn instruction_pointer(&self) -> u64 {
        self.raw.instruction_pointer
    }

    /// The code selector at the point of the trap.
    pub const fn code_selector(&self) -> u64 {
        self.raw.code_selector
    }

    /// The flags register at the point of the trap.
    pub const fn flags(&self) -> u64 {
        self.raw.flags
    }

    /// The stack pointer at the point of the trap.
    pub const fn stack_pointer(&self) -> u64 {
        self.raw.stack_pointer
    }

    /// The stack selector at the point of the trap.
    pub const fn stack_selector(&self) -> u64 {
        self.raw.stack_selector
    }
}

impl fmt::Debug for TrapFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrapFrame")
            .field("r15", &self.r15())
            .field("r14", &self.r14())
            .field("r13", &self.r13())
            .field("r12", &self.r12())
            .field("r11", &self.r11())
            .field("r10", &self.r10())
            .field("r9", &self.r9())
            .field("r8", &self.r8())
            .field("rbp", &self.rbp())
            .field("rdi", &self.rdi())
            .field("rsi", &self.rsi())
            .field("rdx", &self.rdx())
            .field("rcx", &self.rcx())
            .field("rbx", &self.rbx())
            .field("rax", &self.rax())
            .field("vector", &self.vector())
            .field("error_code", &self.error_code())
            .field("instruction_pointer", &self.instruction_pointer())
            .field("code_selector", &self.code_selector())
            .field("flags", &self.flags())
            .field("stack_pointer", &self.stack_pointer())
            .field("stack_selector", &self.stack_selector())
            .finish()
    }
}

/// What the kernel does when a vector is raised.
///
/// Runs with interrupts disabled, on the interrupted stack unless the gate
/// selected one from the task-state segment. Returning resumes the interrupted
/// code, which is meaningful for a trap such as a breakpoint and is a loop for
/// a fault, whose saved instruction pointer is the instruction that faulted.
pub type Handler = fn(&TrapFrame);

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

/// The assembly frame behind the innermost handler's read-only snapshot.
///
/// Its address is the evidence the scheduler needs; the snapshot's private
/// origin is the only safe route back to it.
static LIVE_FRAME: AtomicPtr<RawTrapFrame> = AtomicPtr::new(core::ptr::null_mut());

/// Whether this CPU is inside a trap handler.
pub fn in_trap() -> bool {
    TRAP_DEPTH.load(Ordering::Relaxed) != 0
}

/// Bytes the entry path pushes before the dispatcher is called.
///
/// The pushes and the struct that mirrors them have to agree on how many
/// words that is, and only a live trap can say whether they still do.
pub const SAVED_FRAME_BYTES: u64 = size_of::<RawTrapFrame>() as u64;

/// Where the innermost live handler's assembly frame begins.
///
/// A number rather than a pointer: nothing safe can reach the frame through
/// it, and the machine selftest needs only to subtract.
#[doc(hidden)]
pub fn live_frame_address() -> u64 {
    LIVE_FRAME.load(Ordering::Relaxed) as u64
}

/// Whether `frame` is the one the innermost live handler was entered with.
pub(crate) fn is_live_frame(frame: &TrapFrame) -> bool {
    LIVE_FRAME.load(Ordering::Relaxed) == frame.origin
}

/// Resume the interrupted context at `address` instead of where it faulted.
///
/// `pub(super)`, and there is exactly one caller: `usercopy::recover`, which
/// has already established that the fault was inside its own two-instruction
/// copy and that `address` is its own fixup label. A general "set the
/// instruction pointer" is not something this kernel should have — it is the
/// primitive every exploit wants — and the narrowness is the whole safety
/// argument (D-120).
///
/// Refused unless `frame` is the innermost live one, as [`answer`] is: a stale
/// snapshot's `origin` names a stack another thread is now standing on.
///
/// `false` if the frame is not live, having written nothing.
pub(super) fn resume_at(frame: &TrapFrame, address: u64) -> bool {
    if !is_live_frame(frame) {
        return false;
    }
    // SAFETY: `origin` is the assembly frame the entry path pushed and the
    // check above says it is still the innermost one, so it is on this
    // thread's kernel stack and the stub has not yet popped it.
    let slot = unsafe { core::ptr::addr_of_mut!((*frame.origin).instruction_pointer) };
    // SAFETY: as above. The value is a label inside this crate's own text, so
    // `iretq` returns to an instruction rather than to whatever a caller chose.
    unsafe { slot.write(address) };
    true
}

/// Set the `rax` the interrupted context will resume with.
///
/// The one way anything writes a trap frame, and the whole of the return-value
/// convention: a call's result goes back in `rax`, which the entry stub pops
/// on its way to `iretq`. Everything else in the frame stays read-only —
/// `TrapFrame` is a snapshot on purpose, and a handler that could edit the
/// interrupted state at will is a handler that can resume a thread somewhere
/// it never was (D-116).
///
/// Refused unless `frame` is the innermost live one, for the same reason
/// `switch_from_trap` refuses a foreign frame: a stale snapshot's `origin`
/// names a stack another thread is now standing on.
///
/// `false` if the frame is not live, having written nothing.
pub fn answer(frame: &TrapFrame, value: u64) -> bool {
    if !is_live_frame(frame) {
        return false;
    }
    // SAFETY: `origin` is the assembly frame the entry path pushed and the
    // check above says it is still the innermost one, so it is on this
    // thread's kernel stack and the stub has not yet popped it.
    let slot = unsafe { core::ptr::addr_of_mut!((*frame.origin).rax) };
    // SAFETY: as above, and `rax` is caller-saved — no compiled code is
    // holding the value being replaced.
    unsafe { slot.write(value) };
    true
}

/// Take this context's trap state off the CPU, for a switch to another.
///
/// Paired with [`resume_trap`] on the way back; between them the CPU runs a
/// thread whose depth and live frame are its own.
pub(crate) fn suspend_trap() -> TrapSuspension {
    TrapSuspension {
        depth: TRAP_DEPTH.swap(0, Ordering::Relaxed),
        frame: LIVE_FRAME.swap(core::ptr::null_mut(), Ordering::Relaxed),
    }
}

/// Put back the state [`suspend_trap`] took.
pub(crate) fn resume_trap(suspended: TrapSuspension) {
    TRAP_DEPTH.store(suspended.depth, Ordering::Relaxed);
    LIVE_FRAME.store(suspended.frame, Ordering::Relaxed);
}

/// One context's trap state, parked on its own stack across a switch.
pub(crate) struct TrapSuspension {
    depth: usize,
    frame: *mut RawTrapFrame,
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
pub fn init(handler: Handler, user_gate: Option<u8>) {
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
        let address = (base + vector * STUB_BYTES) as u64;
        // Exactly one gate may be reached from ring 3, and it is named rather
        // than defaulted: every other vector raises a general-protection fault
        // when user code says `int`, because a device's line is not a service
        // (D-96).
        *gate = if user_gate == Some(vector as u8) {
            Gate::callable_from_user(address, segment::KERNEL_CODE, stack)
        } else {
            Gate::interrupt(address, segment::KERNEL_CODE, stack)
        };
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

/// The privilege level that may reach `vector` with `int`.
///
/// Read back from the table the CPU is walking, not from what was asked for.
pub fn gate_privilege(vector: u8) -> u8 {
    // SAFETY: the table is written once by `init` and never again, and this
    // reads one gate of it by value.
    let gates = unsafe { &*IDT.as_mut_ptr() };
    gates[vector as usize].privilege()
}

/// Whether `clac` may be executed on the way in.
///
/// A plain byte for the same reason [`MDS_CLEAR`] is one: the entry stub reads
/// it with a single instruction and nothing writes it after boot. Zero until
/// the kernel has established that this processor has SMAP, because `clac` is
/// a `#UD` without it.
#[unsafe(no_mangle)]
static mut SMAP_ARMED: u8 = 0;

/// Let the entry path clear `AC` on the way in.
///
/// Called once, before ring 3 exists.
///
/// # Panics
///
/// If SMAP is not enabled. `clac` is a `#UD` without it, and the fault
/// handler re-enters the stub that executed it — an unbreakable loop, on a
/// machine configuration this project deliberately supports (D-90). A
/// call-site check is what D-77 rejected: another safe caller can omit it
/// (D-105).
pub fn arm_access_clear() {
    assert!(
        cpu::Protections::enabled().smap,
        "arming the access clear without smap would fault on every trap"
    );
    // SAFETY: written once during boot, before any trap can be taken from
    // ring 3 and on the one CPU there is.
    unsafe { SMAP_ARMED = 1 };
}

/// Whether the entry path clears `AC`.
pub fn access_clear_is_armed() -> bool {
    // SAFETY: a one-byte read of a value only ever written during boot.
    unsafe { SMAP_ARMED == 1 }
}

/// Whether the return path should flush before entering ring 3.
///
/// A plain byte rather than an atomic because the entry stub reads it with one
/// instruction and nothing writes it after boot. Set from `cpuid`, so a
/// processor reporting `MDS_NO` pays nothing.
#[unsafe(no_mangle)]
pub(super) static mut MDS_CLEAR: u8 = 0;

/// Turn on the buffer flush that precedes every return to ring 3.
///
/// Idempotent, and meant to be called once before any user code exists. Where
/// the processor reports `MDS_NO` this is never called and the return path
/// skips the instruction entirely (D-98).
pub fn enable_buffer_clear() {
    // SAFETY: written once during boot, before ring 3 exists and therefore
    // before any return path can read it, on the one CPU there is.
    unsafe { MDS_CLEAR = 1 };
}

/// How many times the buffer flush has actually executed.
///
/// Counted in the asm, beside the `verw` itself. Reading the arming byte says
/// only that the instruction was *compiled in*, which is the split D-101 found
/// after the flush shipped unexecuted for a phase — so the check reads the
/// instruction's own tally instead (D-105).
pub fn buffer_clears() -> u64 {
    let counter = core::ptr::addr_of!(BUFFER_CLEARS);
    // SAFETY: naturally aligned and written only by an `inc` from the one
    // CPU's own asm, so a read sees one value and never a mixture.
    unsafe { counter.read_volatile() }
}

/// The tally [`buffer_clears`] reports, incremented where the `verw` is.
pub(super) static mut BUFFER_CLEARS: u64 = 0;

/// Whether the buffer flush is armed.
pub fn buffer_clear_is_armed() -> bool {
    // SAFETY: a one-byte read of a value only ever written during boot.
    unsafe { MDS_CLEAR == 1 }
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
#[must_use = "dropping this state intentionally leaves interrupts disabled"]
#[derive(Debug, PartialEq, Eq)]
pub struct InterruptState {
    was_enabled: bool,
    stays_here: PhantomData<*mut ()>,
}

impl InterruptState {
    /// Put the interrupt flag back the way it was.
    pub fn restore(mut self) {
        self.restore_in_place();
    }

    /// Restore a state stored inside a guard that is itself being dropped.
    pub(crate) fn restore_in_place(&mut self) {
        if core::mem::replace(&mut self.was_enabled, false) {
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
    InterruptState {
        was_enabled: previous,
        stays_here: PhantomData,
    }
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
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so halting with the flag set would strand it"
    );
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
///
/// # Panics
///
/// Panics if a spin lock is held: the flag is what keeps its holder from
/// being interrupted, and an interrupt that then waits on the same lock waits
/// on a context that cannot resume (D-66).
fn set_flag() {
    assert!(
        crate::sync::spinlocks_held() == 0 || crate::panic::is_panicking(),
        "a spin lock is held, so setting the interrupt flag would strand it"
    );
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
    assert!(
        table_is_loaded(),
        "load the interrupt table before raising a breakpoint"
    );
    // SAFETY: `int3` raises vector 3, whose gate the assert confirmed loaded.
    // `rax` is caller-saved, and exposing the interrupted `rsp` through it
    // lets the machine selftest distinguish a saved word from adjacent stack.
    unsafe { asm!("mov rax, rsp", "int3", out("rax") _) };
}

/// Call the registered handler with the frame the entry path saved.
extern "C" fn dispatch(raw: *mut RawTrapFrame) {
    let registered = HANDLER.load(Ordering::Acquire);
    if registered.is_null() {
        cpu::halt_forever();
    }

    // SAFETY: the slot is written only by `init`, from a `Handler`, and only
    // before the table naming this entry path is loaded. A function pointer and
    // a data pointer are the same width on x86_64.
    let handler: Handler = unsafe { core::mem::transmute(registered) };

    // SAFETY: the entry path passes a properly aligned, fully initialised
    // register block with the exact `RawTrapFrame` layout. Copying it leaves
    // the assembly-owned frame untouched for `iretq`.
    let saved = unsafe { raw.read() };
    let frame = TrapFrame {
        raw: saved,
        origin: raw,
        stays_here: PhantomData,
    };

    TRAP_DEPTH.fetch_add(1, Ordering::Relaxed);
    // Swapped so a nested trap restores its outer frame on the way out.
    let outer = LIVE_FRAME.swap(raw, Ordering::Relaxed);
    handler(&frame);
    LIVE_FRAME.store(outer, Ordering::Relaxed);
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
    // and an interrupt does not clear it. An interrupted stack is not a Rust
    // call boundary, so `rsp` is aligned explicitly; `rbx` is already saved and
    // the called ABI preserves it, making it a stable route back to the frame.
    // Floating-point state is not saved because the kernel is built without
    // SSE and cannot disturb it.
    "wost_interrupt_entry:",
    "cld",
    // And `clac`, because `AC` is *not* one of the flags interrupt delivery
    // clears — and it is writable at CPL 3. A ring-3 thread that sets it walks
    // into this handler with SMAP disabled, and it stays disabled for every
    // thread switched to afterwards, because a context switch restores no
    // flags. That is the mitigation this phase is built around, defeated from
    // the far side of the boundary by three instructions (D-101). Linux emits
    // `ASM_CLAC` at the top of every entry point for exactly this reason.
    //
    // Branched rather than patched: `clac` is a `#UD` where SMAP is absent,
    // which the degraded boot exercises (D-90), and this kernel has no
    // instruction-patching machinery to make it free.
    "cmp byte ptr [rip + {smap}], 0",
    "je 3f",
    "clac",
    "3:",
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
    "mov rbx, rsp",
    "and rsp, -16",
    "call {dispatch}",
    "mov rsp, rbx",
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
    // Returning to ring 3? The saved `CS` is at `[rsp + 8]` now that the
    // vector and error code have been stepped over, and its low two bits are
    // the privilege the CPU will return to.
    "test byte ptr [rsp + 8], 3",
    "jz 2f",
    // `VERW` on a data selector flushes the microarchitectural buffers MDS
    // reads from, and this is the last instruction before the boundary. One
    // instruction, on the path that is painful to modify once mature, which is
    // why D-81 asked for it now — and skipped where the processor says it is
    // not needed (D-98).
    "cmp byte ptr [rip + {clear}], 0",
    "je 2f",
    "sub rsp, 8",
    "mov word ptr [rsp], {kernel_data}",
    "verw word ptr [rsp]",
    "add rsp, 8",
    "inc qword ptr [rip + {clears}]",
    "2:",
    "iretq",
    stub_bytes = const STUB_BYTES,
    clear = sym MDS_CLEAR,
    clears = sym BUFFER_CLEARS,
    smap = sym SMAP_ARMED,
    kernel_data = const (segment::KERNEL_DATA_BITS as u32),
    architectural = const exception::COUNT,
    interrupts = const idt::TABLE_LEN - exception::COUNT as usize,
    error_code_vectors = const exception::ERROR_CODE_VECTORS,
    dispatch = sym dispatch,
);
