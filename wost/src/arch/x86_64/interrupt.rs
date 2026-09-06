//! Interrupt and exception entry.
//!
//! Stable Rust has no interrupt calling convention, so assembly stubs normalize
//! CPU frames before a common tail calls [`Handler`] (D-01).
//!
//! Every gate is an interrupt gate, so a handler starts with interrupts
//! disabled. See `docs/006-cpu-fundamentals.md` for the frame layout argument.
//!
//! Interrupt admission lives here because its safety depends on a loaded table
//! whose gates are all present.

use core::arch::asm;
use core::marker::PhantomData;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use x86_apic::vector::InterruptVector;
use x86_tables::exception;
use x86_tables::idt::{self, Gate};

use super::cpu;
use super::pic;
use super::segment;
use super::table::TablePointer;
use crate::cell::HardwareCell;
use crate::sync::Once;

const CONTEXTS: usize = 32;
const MAX_EXCLUSIONS: usize = usize::BITS as usize;

const _: () = assert!(CONTEXTS == super::task::MAX_TASKS);

struct ExclusionLedger {
    depth: AtomicUsize,
    deferred: AtomicUsize,
    outer_was_enabled: AtomicBool,
}

impl ExclusionLedger {
    const fn new() -> Self {
        Self {
            depth: AtomicUsize::new(0),
            deferred: AtomicUsize::new(0),
            outer_was_enabled: AtomicBool::new(false),
        }
    }
}

static ACTIVE_CONTEXT: AtomicUsize = AtomicUsize::new(super::task::BOOT);
static EXCLUSIONS: [ExclusionLedger; CONTEXTS] = [const { ExclusionLedger::new() }; CONTEXTS];

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
#[derive(Debug)]
pub struct TrapFrame {
    raw: RawTrapFrame,
    origin: *mut RawTrapFrame,
    stays_here: PhantomData<*mut ()>,
}

impl TrapFrame {
    /// Saved `rdi`.
    pub const fn rdi(&self) -> u64 {
        self.raw.rdi
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

/// What the kernel does when a vector is raised.
///
/// Runs with interrupts disabled, on the interrupted stack unless the gate
/// selected a task-state-segment stack. Fault handlers must redirect or stop;
/// their saved instruction pointer still names the faulting instruction.
pub type Handler = fn(&TrapFrame);

static IDT: HardwareCell<[Gate; idt::TABLE_LEN]> =
    HardwareCell::new([Gate::MISSING; idt::TABLE_LEN]);
static HANDLER: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static INIT: Once = Once::new();

/// How many trap handlers are on this CPU's stack — zero in thread context.
///
/// The heap and scheduler consult this count. Context switches park it with
/// the thread that owns the interrupted stack (D-55).
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
#[cfg(feature = "selftest")]
pub const SAVED_FRAME_BYTES: u64 = size_of::<RawTrapFrame>() as u64;

/// Where the innermost live handler's assembly frame begins.
///
/// A number rather than a pointer: nothing safe can reach the frame through
/// it, and the machine selftest needs only to subtract.
#[doc(hidden)]
#[cfg(feature = "selftest")]
pub fn live_frame_address() -> u64 {
    LIVE_FRAME.load(Ordering::Relaxed) as u64
}

/// Whether `frame` is the one the innermost live handler was entered with.
pub(crate) fn is_live_frame(frame: &TrapFrame) -> bool {
    LIVE_FRAME.load(Ordering::Relaxed) == frame.origin
}

/// Resume a faulting user copy at its unforgeable fixup target.
///
/// Refused unless `frame` is the innermost live one, as [`answer`] is: a stale
/// snapshot's `origin` names a stack another thread is now standing on.
///
/// `false` if the frame is not live, having written nothing.
#[must_use = "a refused resume changed no trap frame"]
pub(super) fn resume_usercopy(frame: &TrapFrame, target: super::usercopy::RecoveryTarget) -> bool {
    if !is_live_frame(frame) {
        return false;
    }
    // SAFETY: `origin` is the assembly frame the entry path pushed and the
    // check above says it is still the innermost one, so it is on this
    // thread's kernel stack and the stub has not yet popped it.
    let slot = unsafe { core::ptr::addr_of_mut!((*frame.origin).instruction_pointer) };
    // SAFETY: as above. Only `usercopy` can construct the target, from its
    // in-crate fixup label, so `iretq` cannot be redirected by another caller.
    unsafe { core::ptr::write(slot, target.address()) };
    true
}

/// Resume the controlled write-protection probe at its private fixup target.
#[cfg(feature = "selftest")]
#[must_use = "a refused resume changed no trap frame"]
pub(super) fn resume_write_protection(
    frame: &TrapFrame,
    target: super::cpu::WriteProtectionRecoveryTarget,
) -> bool {
    if !is_live_frame(frame) {
        return false;
    }
    // SAFETY: `origin` is the innermost assembly frame and remains live.
    let slot = unsafe { core::ptr::addr_of_mut!((*frame.origin).instruction_pointer) };
    // SAFETY: only the write-protection probe can construct this in-crate label.
    unsafe { core::ptr::write(slot, target.address()) };
    true
}

/// Set the `rax` the interrupted context will resume with.
///
/// Only the ABI return register is mutable; the remaining interrupted state
/// stays a read-only snapshot (D-116).
///
/// Refused unless `frame` is the innermost live one, for the same reason
/// `switch_from_trap` refuses a foreign frame: a stale snapshot's `origin`
/// names a stack another thread is now standing on.
///
/// `false` if the frame is not live, having written nothing.
#[must_use = "a refused answer changed no trap frame"]
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
    unsafe { core::ptr::write(slot, value) };
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
/// All 256 gates are present so a stray vector reaches the common handler.
/// Does nothing after the first call.
///
/// # Panics
///
/// Panics if the global descriptor table has not been installed — the gates
/// name its code selector — or if the generated stub table is not the size the
/// indexing assumes.
pub fn init(handler: Handler, user_gate: Option<InterruptVector>) {
    if !INIT.claim() {
        return;
    }

    assert!(
        segment::current_code_selector() == segment::KERNEL_CODE,
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
        // Only the named gate is reachable from ring 3; a user `int` at any
        // other vector faults, because a device's line is not a service (D-96).
        *gate = if user_gate.is_some_and(|user| usize::from(user.get()) == vector) {
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
#[cfg(feature = "selftest")]
pub fn gate_privilege(vector: u8) -> u8 {
    // SAFETY: the table is written once by `init` and never again, and this
    // reads one gate of it by value.
    let gates = unsafe { &*IDT.as_mut_ptr() };
    gates[vector as usize].privilege()
}

/// Whether `clac` may be executed on the way in.
///
/// The entry stub needs one immutable byte; `clac` would fault without SMAP.
#[unsafe(no_mangle)]
static mut SMAP_ARMED: u8 = 0;

/// Let the entry path clear `AC` on the way in.
///
/// Called once, before ring 3 exists.
///
/// # Panics
///
/// If SMAP is not enabled. `clac` would fault in the entry stub and re-enter
/// the same path indefinitely; the safe API checks this itself (D-90, D-105).
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
#[cfg(feature = "selftest")]
pub fn access_clear_is_armed() -> bool {
    // SAFETY: a one-byte read of a value only ever written during boot.
    unsafe { SMAP_ARMED == 1 }
}

/// Whether the return path should flush before entering ring 3.
///
/// The entry stub reads this immutable byte directly.
#[unsafe(no_mangle)]
pub(super) static mut MDS_CLEAR: u8 = 0;

pub(super) const RECORD_BUFFER_CLEARS: usize = cfg!(feature = "selftest") as usize;

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
/// Counted conditionally after `verw`, so the tally witnesses execution rather
/// than only an armed branch (D-101, D-105).
#[cfg(feature = "selftest")]
pub fn buffer_clears() -> u64 {
    let counter = core::ptr::addr_of!(BUFFER_CLEARS);
    // SAFETY: naturally aligned and written only by an `inc` from the one
    // CPU's own asm, so a read sees one value and never a mixture.
    unsafe { counter.read_volatile() }
}

/// The tally [`buffer_clears`] reports, incremented where the `verw` is.
#[cfg(feature = "selftest")]
#[unsafe(no_mangle)]
static mut BUFFER_CLEARS: u64 = 0;

/// Whether the buffer flush is armed.
#[cfg(feature = "selftest")]
pub fn buffer_clear_is_armed() -> bool {
    // SAFETY: a one-byte read of a value only ever written during boot.
    unsafe { MDS_CLEAR == 1 }
}

fn idt_is_loaded() -> bool {
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

/// Whether the CPU's interrupt descriptor table register points at the table
/// [`init`] built, with the limit it wrote.
#[cfg(feature = "selftest")]
pub fn table_is_loaded() -> bool {
    idt_is_loaded()
}

/// One context's place in the interrupt-exclusion stack.
///
/// Context switches park this state with the stack that owns it. Restoring an
/// outer state first defers it until every inner state has unwound; it never
/// enables interrupts through a still-live exclusion.
#[must_use = "dropping this state intentionally leaves interrupts disabled"]
#[derive(Debug, PartialEq, Eq)]
pub struct InterruptState {
    owner: usize,
    level: usize,
    live: bool,
    stays_here: PhantomData<*mut ()>,
}

impl InterruptState {
    /// Put the interrupt flag back the way it was.
    pub fn restore(mut self) {
        self.restore_in_place();
    }

    /// Restore a state stored inside a guard that is itself being dropped.
    pub(crate) fn restore_in_place(&mut self) {
        if !core::mem::replace(&mut self.live, false) {
            return;
        }
        if restore_level(self.owner, self.level) {
            set_flag();
        }
    }

    /// Wait atomically while retaining this outer exclusion.
    pub fn wait_for_interrupt(&mut self) {
        assert!(self.live, "an interrupt exclusion was already consumed");
        assert_owner(self.owner);
        let ledger = &EXCLUSIONS[self.owner];
        assert_eq!(self.level, 1, "only an outer exclusion may wait");
        assert_eq!(
            ledger.depth.load(Ordering::Relaxed),
            1,
            "an inner interrupt exclusion is still live"
        );
        assert_eq!(
            ledger.deferred.load(Ordering::Relaxed),
            0,
            "a deferred interrupt restoration is still live"
        );
        assert!(
            ledger.outer_was_enabled.load(Ordering::Relaxed),
            "a context entered with interrupts disabled cannot wait by enabling them"
        );
        assert_platform_enable_preconditions();

        ledger.depth.store(0, Ordering::Relaxed);
        ledger.outer_was_enabled.store(false, Ordering::Relaxed);
        // SAFETY: `sti; hlt` closes the lost-wakeup window and `cli`
        // re-establishes this token's exclusion before compiled code resumes.
        unsafe { asm!("sti", "hlt", "cli", options(nostack)) };
        assert_owner(self.owner);
        assert_eq!(
            ledger.depth.load(Ordering::Relaxed),
            0,
            "an interrupt handler returned with an exclusion live"
        );
        ledger.outer_was_enabled.store(true, Ordering::Relaxed);
        ledger.depth.store(1, Ordering::Relaxed);
    }

    /// Consume the sole exclusion while deliberately leaving interrupts off.
    pub(crate) fn commit_disabled(mut self) {
        assert!(self.live, "an interrupt exclusion was already consumed");
        assert_owner(self.owner);
        let ledger = &EXCLUSIONS[self.owner];
        assert_eq!(self.level, 1, "an outer interrupt exclusion is still live");
        assert_eq!(
            ledger.depth.load(Ordering::Relaxed),
            1,
            "an inner interrupt exclusion is still live"
        );
        assert_eq!(ledger.deferred.load(Ordering::Relaxed), 0);
        ledger.depth.store(0, Ordering::Relaxed);
        ledger.outer_was_enabled.store(false, Ordering::Relaxed);
        self.live = false;
    }

    /// End a dying context's exclusion ownership without setting its flag.
    pub(super) fn commit_context_disabled(mut self) {
        assert!(self.live, "an interrupt exclusion was already consumed");
        assert_owner(self.owner);
        let ledger = &EXCLUSIONS[self.owner];
        assert_eq!(
            self.level,
            ledger.depth.load(Ordering::Relaxed),
            "an inner interrupt exclusion is still live"
        );
        ledger.depth.store(0, Ordering::Relaxed);
        ledger.deferred.store(0, Ordering::Relaxed);
        ledger.outer_was_enabled.store(false, Ordering::Relaxed);
        self.live = false;
    }

    /// Whether the context accepted interrupts before this exclusion.
    pub(super) fn were_enabled(&self) -> bool {
        self.live
            && self.level == 1
            && EXCLUSIONS[self.owner]
                .outer_was_enabled
                .load(Ordering::Relaxed)
    }

    /// Transfer this state out of a guard without registering a replacement.
    #[cfg(feature = "selftest")]
    pub(crate) fn take_for_transfer(&mut self) -> Self {
        core::mem::replace(
            self,
            Self {
                owner: usize::MAX,
                level: 0,
                live: false,
                stays_here: PhantomData,
            },
        )
    }
}

fn assert_owner(owner: usize) {
    assert_eq!(
        ACTIVE_CONTEXT.load(Ordering::Relaxed),
        owner,
        "an interrupt exclusion belongs to another task context"
    );
}

fn restore_level(owner: usize, level: usize) -> bool {
    assert_owner(owner);
    let ledger = &EXCLUSIONS[owner];
    let depth = ledger.depth.load(Ordering::Relaxed);
    assert!(
        level != 0 && level <= depth,
        "interrupt exclusion order is corrupt"
    );
    let bit = 1usize << (level - 1);
    let mut deferred = ledger.deferred.load(Ordering::Relaxed);
    assert_eq!(deferred & bit, 0, "interrupt exclusion restored twice");
    if level != depth {
        ledger.deferred.store(deferred | bit, Ordering::Relaxed);
        return false;
    }

    let mut remaining = depth - 1;
    while remaining != 0 {
        let outer = 1usize << (remaining - 1);
        if deferred & outer == 0 {
            break;
        }
        deferred &= !outer;
        remaining -= 1;
    }
    ledger.deferred.store(deferred, Ordering::Relaxed);
    ledger.depth.store(remaining, Ordering::Relaxed);
    if remaining == 0 {
        assert_eq!(deferred, 0, "a restoration survived its interrupt context");
        ledger.outer_was_enabled.swap(false, Ordering::Relaxed)
    } else {
        false
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
    unsafe { asm!("cli", options(nostack)) };
    let owner = ACTIVE_CONTEXT.load(Ordering::Relaxed);
    let ledger = &EXCLUSIONS[owner];
    let depth = ledger.depth.load(Ordering::Relaxed);
    assert!(
        depth < MAX_EXCLUSIONS,
        "interrupt exclusion nesting exceeds one machine word"
    );
    if depth == 0 {
        assert_eq!(ledger.deferred.load(Ordering::Relaxed), 0);
        ledger.outer_was_enabled.store(previous, Ordering::Relaxed);
    } else {
        assert!(!previous, "interrupts became enabled inside an exclusion");
    }
    ledger.depth.store(depth + 1, Ordering::Relaxed);
    InterruptState {
        owner,
        level: depth + 1,
        live: true,
        stays_here: PhantomData,
    }
}

/// Begin accepting maskable interrupts on this CPU.
///
/// # Panics
///
/// Panics unless the table is loaded and the legacy controller is silent;
/// either omission can route the first interrupt into a reset.
pub fn enable() {
    assert_enable_preconditions();
    set_flag();
}

fn assert_enable_preconditions() {
    assert_platform_enable_preconditions();
    assert_eq!(
        EXCLUSIONS[ACTIVE_CONTEXT.load(Ordering::Relaxed)]
            .depth
            .load(Ordering::Relaxed),
        0,
        "an interrupt exclusion is still live"
    );
}

fn assert_platform_enable_preconditions() {
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so setting the interrupt flag would strand it"
    );
    assert!(
        idt_is_loaded(),
        "load the interrupt table before accepting interrupts"
    );
    assert!(
        pic::is_silenced(),
        "silence the legacy controller before accepting interrupts"
    );
}

/// Set the interrupt flag with nothing checked.
///
/// Private because only restoration may reuse a previously admitted flag.
///
/// # Panics
///
/// Panics if a spin lock is held: the flag is what keeps its holder from
/// being interrupted, and an interrupt that then waits on the same lock waits
/// on a context that cannot resume (D-66).
fn set_flag() {
    assert_eq!(
        EXCLUSIONS[ACTIVE_CONTEXT.load(Ordering::Relaxed)]
            .depth
            .load(Ordering::Relaxed),
        0,
        "an interrupt exclusion is still live"
    );
    assert!(
        crate::sync::spinlocks_held() == 0 || crate::panic::is_panicking(),
        "a spin lock is held, so setting the interrupt flag would strand it"
    );
    // SAFETY: `sti` sets IF on the current CPU and touches nothing else. Every
    // caller either checked the preconditions or is restoring a flag whose
    // caller did.
    unsafe { asm!("sti", options(nostack)) };
}

/// Move interrupt-exclusion ownership at the context-switch boundary.
pub(super) fn switch_context(to: usize) {
    assert!(
        !are_enabled(),
        "a context switch requires interrupts disabled"
    );
    assert!(to < CONTEXTS, "task context is outside the exclusion table");
    ACTIVE_CONTEXT.store(to, Ordering::Relaxed);
}

/// Whether `slot` has no parked interrupt exclusions.
pub(super) fn context_is_clear(slot: usize) -> bool {
    slot < CONTEXTS && EXCLUSIONS[slot].depth.load(Ordering::Relaxed) == 0
}

/// Prove nested restoration and direct admission share the production ledger.
#[cfg(feature = "selftest")]
pub fn exclusion_order_is_enforced_for_test() -> bool {
    let owner = ACTIVE_CONTEXT.load(Ordering::Relaxed);
    if !are_enabled() || !context_is_clear(owner) {
        return false;
    }
    let outer = disable();
    let inner = disable();
    outer.restore();
    let deferred = !are_enabled() && EXCLUSIONS[owner].depth.load(Ordering::Relaxed) == 2;
    let direct_enable_refused = EXCLUSIONS[owner].depth.load(Ordering::Relaxed) != 0;
    inner.restore();
    deferred && direct_enable_refused && are_enabled()
}

/// Whether interrupt ownership follows `slot` on this instruction stream.
#[cfg(feature = "selftest")]
pub fn context_matches_for_test(slot: usize) -> bool {
    ACTIVE_CONTEXT.load(Ordering::Relaxed) == slot
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
#[cfg(feature = "selftest")]
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
    let saved = unsafe { core::ptr::read(raw) };
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

::core::arch::global_asm!(
    // Stubs are padded to one size so the table indexes by vector. Vectors
    // without an error code push a zero, so every frame has one shape;
    // `ERROR_CODE_VECTORS` is the one source, checked against the SDM on the host.
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

    // Entry clears neither DF nor the alignment debt. RBX survives the call
    // and routes back to the frame; kernel code holds no floating-point state.
    "wost_interrupt_entry:",
    "cld",
    // Ring 3 controls `AC` and entry does not clear it, so every SMAP entry
    // does (D-101); a branch, because `clac` faults without SMAP.
    "cmp byte ptr [rip + {smap}], 0",
    "je 3f",
    "clac",
    "3:",
    // A privilege-changing entry nulls live `SS`; dispatch may switch kernel
    // contexts before `iretq`, so restore the kernel selector before Rust.
    "push rax",
    "mov ax, {kernel_data}",
    "mov ss, ax",
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
    // Returning to ring 3? With vector and error code stepped over, the saved
    // `CS` sits at `[rsp + 8]` and its low two bits are the target privilege.
    "test byte ptr [rsp + 8], 3",
    "jz 2f",
    // `VERW` flushes MDS-visible buffers as the last instruction before the
    // boundary, unless `MDS_NO` makes it pointless (D-81, D-98).
    "cmp byte ptr [rip + {clear}], 0",
    "je 2f",
    "sub rsp, 8",
    "mov word ptr [rsp], {kernel_data}",
    "verw word ptr [rsp]",
    ".if {record_clears}",
    "jnz 3f",
    "inc qword ptr [rip + BUFFER_CLEARS]",
    "3:",
    ".endif",
    "add rsp, 8",
    "2:",
    "iretq",
    stub_bytes = const STUB_BYTES,
    clear = sym MDS_CLEAR,
    record_clears = const RECORD_BUFFER_CLEARS,
    smap = sym SMAP_ARMED,
    kernel_data = const (segment::KERNEL_DATA_BITS as u32),
    architectural = const exception::COUNT,
    interrupts = const idt::TABLE_LEN - exception::COUNT as usize,
    error_code_vectors = const exception::ERROR_CODE_VECTORS,
    dispatch = sym dispatch,
);
