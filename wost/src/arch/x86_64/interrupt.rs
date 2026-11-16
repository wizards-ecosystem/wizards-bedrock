use core::arch::asm;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU16, AtomicUsize, Ordering};

use trap_cause::{Fault, TrapCause};
use x86_apic::vector::InterruptVector;
use x86_paging::Flags;
use x86_tables::exception;
use x86_tables::idt::{self, Gate};
pub use x86_tables::return_state::UserReturnError;
use x86_tables::return_state::UserReturnState;

use super::cpu;
use super::pic;
use super::segment;
use super::table::TablePointer;
use crate::cell::HardwareCell;
use crate::sync::{Once, ThreadContext};

const CONTEXTS: usize = 32;
const MAX_EXCLUSIONS: usize = usize::BITS as usize;

const _: () = assert!(
    CONTEXTS == super::task::MAX_TASKS,
    "every task needs its own exclusion context"
);

#[derive(Debug)]
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

pub(crate) const FLAGS_INTERRUPT: u64 = 1 << 9;

const STUB_BYTES: usize = 16;

// Field order mirrors the entry pushes, lowest address first.
#[derive(Clone, Copy, Debug)]
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
    assert!(
        size_of::<RawTrapFrame>() == 22 * size_of::<u64>(),
        "the trap frame layout must match the entry assembly"
    );
    assert!(
        core::mem::offset_of!(RawTrapFrame, r15) == 0,
        "the trap frame layout must match the entry assembly"
    );
    assert!(
        core::mem::offset_of!(RawTrapFrame, r8) == 7 * size_of::<u64>(),
        "the trap frame layout must match the entry assembly"
    );
    assert!(
        core::mem::offset_of!(RawTrapFrame, rax) == 14 * size_of::<u64>(),
        "the trap frame layout must match the entry assembly"
    );
    assert!(
        core::mem::offset_of!(RawTrapFrame, vector) == 15 * size_of::<u64>(),
        "the trap frame layout must match the entry assembly"
    );
    assert!(
        core::mem::offset_of!(RawTrapFrame, flags) == 19 * size_of::<u64>(),
        "the trap frame layout must match the entry assembly"
    );
    assert!(
        core::mem::offset_of!(RawTrapFrame, stack_selector) == 21 * size_of::<u64>(),
        "the trap frame layout must match the entry assembly"
    );
};

#[derive(Debug)]
pub struct TrapFrame {
    raw: RawTrapFrame,
    origin: *mut RawTrapFrame,
    stays_here: PhantomData<*mut ()>,
}

impl TrapFrame {
    pub const fn vector(&self) -> u8 {
        // Lossless: the stub generator below pushes only IDT indices, asserted to fit a byte.
        self.raw.vector as u8
    }

    pub const fn error_code(&self) -> u64 {
        self.raw.error_code
    }

    pub const fn instruction_pointer(&self) -> u64 {
        self.raw.instruction_pointer
    }

    pub const fn code_selector(&self) -> u64 {
        self.raw.code_selector
    }

    pub const fn flags(&self) -> u64 {
        self.raw.flags
    }

    pub const fn stack_pointer(&self) -> u64 {
        self.raw.stack_pointer
    }

    pub const fn is_from_user(&self) -> bool {
        self.raw.code_selector & PRIVILEGE_MASK == USER_PRIVILEGE
    }

    pub const fn was_interruptible(&self) -> bool {
        self.raw.flags & FLAGS_INTERRUPT != 0
    }
}

const PRIVILEGE_MASK: u64 = 0b11;

const USER_PRIVILEGE: u64 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UserEntry {
    Call,
    Fault(TrapCause),
    Interrupted,
}

// The running thread's own ring 3 frame at its kernel stack top; validated before `iretq`.
#[derive(Debug)]
pub struct UserRegisters<'frame> {
    raw: &'frame mut RawTrapFrame,
    stays_here: PhantomData<*mut ()>,
}

impl UserRegisters<'_> {
    pub const fn rax(&self) -> u64 {
        self.raw.rax
    }

    pub const fn rdi(&self) -> u64 {
        self.raw.rdi
    }

    pub const fn rsi(&self) -> u64 {
        self.raw.rsi
    }

    pub const fn rdx(&self) -> u64 {
        self.raw.rdx
    }

    pub const fn r10(&self) -> u64 {
        self.raw.r10
    }

    pub const fn r8(&self) -> u64 {
        self.raw.r8
    }

    pub const fn r9(&self) -> u64 {
        self.raw.r9
    }

    pub const fn vector(&self) -> u8 {
        // Lossless: the stub generator pushes only IDT indices, asserted to fit a byte.
        self.raw.vector as u8
    }

    pub const fn instruction_pointer(&self) -> u64 {
        self.raw.instruction_pointer
    }

    pub const fn flags(&self) -> u64 {
        self.raw.flags
    }

    pub const fn stack_pointer(&self) -> u64 {
        self.raw.stack_pointer
    }

    pub const fn set_return(&mut self, value: u64) {
        self.raw.rax = value;
    }
}

// Fault handlers must redirect or stop: the saved instruction pointer names the faulting one.
pub type Handler = fn(&TrapFrame);

pub type UserEntryHandler = fn(&mut ThreadContext, &mut UserRegisters<'_>, UserEntry);

pub type InvalidReturnHandler = fn(&mut ThreadContext, &UserRegisters<'_>, UserReturnError) -> !;

#[derive(Debug)]
pub struct Handlers {
    on_trap: Handler,
    on_user_entry: UserEntryHandler,
    on_invalid_return: InvalidReturnHandler,
}

impl Handlers {
    pub const fn new(
        on_trap: Handler,
        on_user_entry: UserEntryHandler,
        on_invalid_return: InvalidReturnHandler,
    ) -> Self {
        Self {
            on_trap,
            on_user_entry,
            on_invalid_return,
        }
    }
}

#[unsafe(link_section = ".kpti.tables.idt")]
static IDT: HardwareCell<[Gate; idt::TABLE_LEN]> =
    HardwareCell::new([Gate::MISSING; idt::TABLE_LEN]);
static HANDLERS: AtomicPtr<Handlers> = AtomicPtr::new(core::ptr::null_mut());
static INIT: Once = Once::new();

const NO_USER_GATE: u16 = u16::MAX;

static USER_GATE: AtomicU16 = AtomicU16::new(NO_USER_GATE);

static TRAP_DEPTH: AtomicUsize = AtomicUsize::new(0);

static LIVE_FRAME: AtomicPtr<RawTrapFrame> = AtomicPtr::new(core::ptr::null_mut());

pub(crate) fn in_trap() -> bool {
    TRAP_DEPTH.load(Ordering::Relaxed) != 0
}

pub(crate) fn is_live_frame(frame: &TrapFrame) -> bool {
    LIVE_FRAME.load(Ordering::Relaxed) == frame.origin
}

#[must_use = "a refused resume changed no trap frame"]
pub(super) fn resume_usercopy(frame: &TrapFrame, target: super::usercopy::RecoveryTarget) -> bool {
    if !is_live_frame(frame) {
        return false;
    }
    // SAFETY: `origin` is the pushed assembly frame, still the innermost by the check above, so it
    // is on this thread's kernel stack and the stub has not yet popped it.
    let slot = unsafe { core::ptr::addr_of_mut!((*frame.origin).instruction_pointer) };
    // SAFETY: as above. Only `usercopy` can construct the target, from its
    // in-crate fixup label, so `iretq` cannot be redirected by another caller.
    unsafe { core::ptr::write(slot, target.address()) };
    true
}

pub(crate) fn suspend_trap() -> TrapSuspension {
    TrapSuspension {
        depth: TRAP_DEPTH.swap(0, Ordering::Relaxed),
        frame: LIVE_FRAME.swap(core::ptr::null_mut(), Ordering::Relaxed),
    }
}

pub(crate) fn resume_trap(suspended: TrapSuspension) {
    TRAP_DEPTH.store(suspended.depth, Ordering::Relaxed);
    LIVE_FRAME.store(suspended.frame, Ordering::Relaxed);
}

#[derive(Debug)]
pub(crate) struct TrapSuspension {
    depth: usize,
    frame: *mut RawTrapFrame,
}

pub fn init(handlers: &'static Handlers, user_gate: Option<InterruptVector>) {
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

    HANDLERS.store(core::ptr::from_ref(handlers).cast_mut(), Ordering::Release);
    if let Some(user) = user_gate {
        USER_GATE.store(u16::from(user.get()), Ordering::Release);
    }

    // SAFETY: `INIT` admitted this caller alone and only this module reaches the table, so no other
    // reference exists, and the CPU is not pointed at it until the `lidt` below.
    let gates = unsafe { &mut *IDT.as_mut_ptr() };
    for (vector, gate) in gates.iter_mut().enumerate() {
        let stack = if vector == exception::DOUBLE_FAULT as usize {
            segment::DOUBLE_FAULT_STACK
        } else {
            0
        };
        let address = (base + vector * STUB_BYTES) as u64;
        // Only the named gate is reachable from ring 3; a user `int` at any other vector faults.
        *gate = if user_gate.is_some_and(|user| usize::from(user.get()) == vector) {
            Gate::callable_from_user(address, segment::KERNEL_CODE, stack)
        } else {
            Gate::interrupt(address, segment::KERNEL_CODE, stack)
        };
    }

    let pointer = TablePointer::new(IDT.address(), idt::TABLE_LIMIT);
    // SAFETY: `lidt` loads the table its ten-byte operand describes, every gate just written
    // present with a valid code selector and a handler address inside the stub table.
    unsafe {
        asm!(
            "lidt [{pointer}]",
            pointer = in(reg) &raw const pointer,
            options(readonly, nostack, preserves_flags),
        );
    }
}

// The entry stub reads this byte directly; `clac` would fault without SMAP.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".kpti.state.route")]
static SMAP_ARMED: AtomicU8 = AtomicU8::new(0);

pub(super) fn arm_access_clear() {
    assert!(
        cpu::Protections::enabled().smap,
        "arming the access clear without smap would fault on every trap"
    );
    SMAP_ARMED.store(1, Ordering::Release);
}

pub(super) fn access_clear_is_armed() -> bool {
    SMAP_ARMED.load(Ordering::Acquire) != 0
}

// The return stub reads this byte directly.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".kpti.state.route")]
static MDS_CLEAR: AtomicU8 = AtomicU8::new(0);

// Read-only in the stripped root, so `verw` writes no scratch byte onto the scrubbed stack.
#[used]
#[unsafe(link_section = ".kpti.rodata.verw_selector")]
static KPTI_VERW_SELECTOR: u16 = segment::KERNEL_DATA_BITS;

pub fn enable_buffer_clear() {
    MDS_CLEAR.store(1, Ordering::Release);
}

fn idt_is_loaded() -> bool {
    let mut pointer = TablePointer::empty();
    // SAFETY: `sidt` stores ten bytes at the operand, a `TablePointer` this call owns.
    unsafe {
        asm!(
            "sidt [{pointer}]",
            pointer = in(reg) &raw mut pointer,
            options(nostack, preserves_flags),
        );
    }
    pointer.base() == IDT.address() && pointer.limit() == idt::TABLE_LIMIT
}

#[must_use = "dropping this state intentionally leaves interrupts disabled"]
#[derive(PartialEq, Eq, Debug)]
pub struct InterruptState {
    owner: usize,
    level: usize,
    live: bool,
    stays_here: PhantomData<*mut ()>,
}

impl InterruptState {
    pub fn restore(mut self) {
        self.restore_in_place();
    }

    pub(crate) fn restore_in_place(&mut self) {
        if !core::mem::replace(&mut self.live, false) {
            return;
        }
        if restore_level(self.owner, self.level) {
            set_flag();
        }
    }

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
        // SAFETY: `sti; hlt` closes the lost-wakeup window and `cli` re-establishes this token's
        // exclusion before compiled code resumes.
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
        assert_eq!(
            ledger.deferred.load(Ordering::Relaxed),
            0,
            "a deferred interrupt restoration is still live"
        );
        ledger.depth.store(0, Ordering::Relaxed);
        ledger.outer_was_enabled.store(false, Ordering::Relaxed);
        self.live = false;
    }

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

    pub fn were_enabled(&self) -> bool {
        self.live
            && self.level == 1
            && EXCLUSIONS[self.owner]
                .outer_was_enabled
                .load(Ordering::Relaxed)
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

pub fn are_enabled() -> bool {
    flags() & FLAGS_INTERRUPT != 0
}

pub(crate) fn disable_for_good() {
    // SAFETY: `cli` clears IF on this CPU, touching nothing else, with no ring 0 precondition.
    unsafe { asm!("cli", options(nostack)) };
}

pub fn disable() -> InterruptState {
    let previous = are_enabled();
    // SAFETY: `cli` clears IF on this CPU, touching nothing else, with no ring 0 precondition;
    // `preserves_flags` is deliberately absent, since modifying IF is the point.
    unsafe { asm!("cli", options(nostack)) };
    let owner = ACTIVE_CONTEXT.load(Ordering::Relaxed);
    let ledger = &EXCLUSIONS[owner];
    let depth = ledger.depth.load(Ordering::Relaxed);
    assert!(
        depth < MAX_EXCLUSIONS,
        "interrupt exclusion nesting exceeds one machine word"
    );
    if depth == 0 {
        assert_eq!(
            ledger.deferred.load(Ordering::Relaxed),
            0,
            "a deferred restoration outlived its exclusions"
        );
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

pub fn enable() {
    assert_platform_enable_preconditions();
    assert_eq!(
        EXCLUSIONS[ACTIVE_CONTEXT.load(Ordering::Relaxed)]
            .depth
            .load(Ordering::Relaxed),
        0,
        "an interrupt exclusion is still live"
    );
    set_flag();
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

// Private: only restoration may reuse a previously admitted flag.
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
    // SAFETY: `sti` sets IF on this CPU and touches nothing else; every caller checked the
    // preconditions or restores a flag whose caller did.
    unsafe { asm!("sti", options(nostack)) };
}

pub(super) fn switch_context(to: usize) {
    assert!(
        !are_enabled(),
        "a context switch requires interrupts disabled"
    );
    assert!(to < CONTEXTS, "task context is outside the exclusion table");
    ACTIVE_CONTEXT.store(to, Ordering::Relaxed);
}

pub(super) fn context_is_clear(slot: usize) -> bool {
    slot < CONTEXTS && EXCLUSIONS[slot].depth.load(Ordering::Relaxed) == 0
}

fn flags() -> u64 {
    let value: u64;
    // SAFETY: `pop` takes back the eight bytes `pushfq` pushed, so the stack ends as it started;
    // `nomem` is accurate, since nothing outside the asm can observe the slot in between.
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

extern "C" fn dispatch(raw: *mut RawTrapFrame) {
    let registered = HANDLERS.load(Ordering::Acquire);
    if registered.is_null() {
        cpu::halt_forever();
    }

    // SAFETY: `init` stores only a statically lived `Handlers` reference before loading the IDT,
    // and never changes it afterwards.
    let handlers = unsafe { &*registered };

    // SAFETY: entry passes an aligned, fully initialised block with the exact `RawTrapFrame`
    // layout, and copying it leaves the assembly-owned frame untouched for `iretq`.
    let saved = unsafe { core::ptr::read(raw) };
    let frame = TrapFrame {
        raw: saved,
        origin: raw,
        stays_here: PhantomData,
    };

    if !frame.is_from_user() {
        trap(handlers, &frame);
        return;
    }
    let entry = user_entry(&frame).unwrap_or_else(|| {
        trap(handlers, &frame);
        UserEntry::Interrupted
    });
    hand_off(handlers, raw, entry);
}

fn trap(handlers: &Handlers, frame: &TrapFrame) {
    TRAP_DEPTH.fetch_add(1, Ordering::Relaxed);
    // Swapped so a nested trap restores its outer frame on the way out.
    let outer = LIVE_FRAME.swap(frame.origin, Ordering::Relaxed);
    (handlers.on_trap)(frame);
    LIVE_FRAME.store(outer, Ordering::Relaxed);
    TRAP_DEPTH.fetch_sub(1, Ordering::Relaxed);
}

// `None` is an interrupt, or a fault no program survives: both take the trap path first.
fn user_entry(frame: &TrapFrame) -> Option<UserEntry> {
    if u16::from(frame.vector()) == USER_GATE.load(Ordering::Acquire) {
        return Some(UserEntry::Call);
    }
    match TrapCause::decode(frame.vector(), frame.error_code()) {
        TrapCause::Fault(Fault::DoubleFault | Fault::MachineCheck | Fault::NonMaskable)
        | TrapCause::Interrupt(_)
        | TrapCause::Reserved(_) => None,
        cause @ TrapCause::Fault(_) => Some(UserEntry::Fault(cause)),
    }
}

// Ring 3 enters only from trap depth zero, so the handler runs as its thread, interrupts on.
fn hand_off(handlers: &Handlers, raw: *mut RawTrapFrame, entry: UserEntry) {
    assert!(!in_trap(), "a ring 3 entry arrived inside a trap");
    // SAFETY: the entry stub passes this thread's frame at its own kernel stack top, aligned and
    // initialised; nested traps push below it and nothing else names it until the return tail.
    let frame = unsafe { &mut *raw };
    let mut registers = UserRegisters {
        raw: frame,
        stays_here: PhantomData,
    };
    let mut context = ThreadContext::mint();
    enable();
    (handlers.on_user_entry)(&mut context, &mut registers, entry);

    // Off from the check to `iretq`, so ring 3 resumes exactly the image accepted here.
    let disabled = disable();
    match invalid_user_return(registers.raw) {
        None => disabled.commit_disabled(),
        Some(error) => {
            disabled.restore();
            (handlers.on_invalid_return)(&mut context, &registers, error)
        }
    }
}

fn invalid_user_return(frame: &RawTrapFrame) -> Option<UserReturnError> {
    let state = UserReturnState::new(
        frame.instruction_pointer,
        frame.code_selector,
        frame.flags,
        frame.stack_pointer,
        frame.stack_selector,
    );
    if let Err(error) = state.validate(
        segment::USER_CODE,
        segment::USER_DATA,
        super::paging::USER_END,
    ) {
        return Some(error);
    }
    match super::paging::translate(frame.instruction_pointer) {
        Some((_, flags)) if flags.contains(Flags::USER) && !flags.contains(Flags::NO_EXECUTE) => {
            None
        }
        _ => Some(UserReturnError::InstructionNotExecutable(
            frame.instruction_pointer,
        )),
    }
}

unsafe extern "C" {
    #[link_name = "wost_interrupt_stubs"]
    fn stub_table();

    #[link_name = "wost_interrupt_stubs_end"]
    fn stub_table_end();

    fn wost_leave_for_user(entry: u64, stack_top: u64) -> !;
}

/// # Safety
///
/// `entry` must be user-executable, `stack_top` must be user-writable, and the live task must have
/// a completely installed shared or paired entry route.
pub(super) unsafe fn leave_for_user(entry: u64, stack_top: u64) -> ! {
    // SAFETY: the caller supplies the mapping and route facts the assembly boundary requires, and
    // the boundary consumes both values before changing roots.
    unsafe { wost_leave_for_user(entry, stack_top) }
}

const _: () = assert!(idt::TABLE_LEN <= 256, "the IDT holds at most 256 vectors");

// The stop path's marker, `[error] wost: kpti stop`, goes straight to the first serial port.
const COM1: u16 = 0x3F8;

::core::arch::global_asm!(
    ".pushsection .kpti.text,\"ax\",@progbits",
    // Stubs are padded to one size so the table indexes by vector; a zero stands in for no code.
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

    // The frame is saved before any general register is used; it is all the stripped root sees.
    "wost_interrupt_entry:",
    "cld",
    // Ring 3 controls `AC` and entry keeps it; `clac` sits behind a branch, faulting without SMAP.
    "cmp byte ptr [rip + {smap}], 0",
    "je 3f",
    "clac",
    "3:",
    // A privilege change nulls `SS`, and dispatch may switch contexts, so reload it before Rust.
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
    // A published route admits two roots: user entry arrives stripped and switches before Rust.
    "cmp qword ptr [rip + WOST_KPTI_ROUTE + {route_state}], 1",
    "je 14f",
    "cmp qword ptr [rip + WOST_KPTI_ROUTE + {route_state}], 2",
    "je wost_kpti_stop",
    "jmp 4f",
    "14:",
    "mov r11, cr3",
    "shr r11, 12",
    "shl r11, 12",
    "cmp r11, qword ptr [rip + WOST_KPTI_ROUTE + {route_stripped}]",
    "je 5f",
    "cmp r11, qword ptr [rip + WOST_KPTI_ROUTE + {route_full}]",
    "jne wost_kpti_load_full_and_stop",
    "test byte ptr [rsp + {frame_cs}], 3",
    "jnz wost_kpti_stop",
    // A kernel frame on either enclave stack is a nested entry inside a root transition.
    "mov r10, qword ptr [rip + WOST_KPTI_ROUTE + {route_transition_stack}]",
    "cmp rsp, r10",
    "jae 12f",
    "sub r10, {entry_stack_bytes}",
    "cmp rsp, r10",
    "jae wost_kpti_stop",
    "12:",
    // A double fault runs on the interrupt stack by design.
    "cmp qword ptr [rsp + {frame_vector}], {double_fault}",
    "je 13f",
    "mov r10, qword ptr [rip + WOST_KPTI_ROUTE + {route_interrupt_stack}]",
    "cmp rsp, r10",
    "jae 13f",
    "sub r10, {entry_stack_bytes}",
    "cmp rsp, r10",
    "jae wost_kpti_stop",
    "13:",
    "jmp 4f",
    "5:",
    ".globl wost_kpti_entry_load_full_root",
    "wost_kpti_entry_load_full_root:",
    "mov r11, qword ptr [rip + WOST_KPTI_ROUTE + {route_full}]",
    "mov cr3, r11",
    // A kernel-origin entry here faulted after the stripped-root load: stop without Rust.
    "test byte ptr [rsp + {frame_cs}], 3",
    "jz wost_kpti_stop",
    // Move the frame off, then scrub both enclave stacks the stripped root could have used.
    "mov rsi, rsp",
    "mov rdi, qword ptr [rip + WOST_KPTI_ROUTE + {route_task_stack}]",
    "sub rdi, {raw_frame_bytes}",
    "mov rbx, rdi",
    "mov rcx, {raw_frame_words}",
    "rep movsq",
    "xor eax, eax",
    "mov rdi, qword ptr [rip + WOST_KPTI_ROUTE + {route_transition_stack}]",
    "sub rdi, {entry_stack_bytes}",
    "mov rcx, {entry_stack_words}",
    "rep stosq",
    "mov rdi, qword ptr [rip + WOST_KPTI_ROUTE + {route_interrupt_stack}]",
    "sub rdi, {entry_stack_bytes}",
    "mov rcx, {entry_stack_words}",
    "rep stosq",
    "mov rsp, rbx",
    "4:",
    "mov rdi, rsp",
    "mov rbx, rsp",
    "and rsp, -16",
    ".globl wost_kpti_entry_call_rust",
    "wost_kpti_entry_call_rust:",
    "call {dispatch}",
    "mov rsp, rbx",
    ".globl wost_interrupt_return",
    "wost_interrupt_return:",
    // A validated paired return copies only registers and the iret frame to scrubbed stacks.
    "test byte ptr [rsp + {frame_cs}], 3",
    "jz 6f",
    "cmp qword ptr [rip + WOST_KPTI_ROUTE + {route_state}], 1",
    "je 15f",
    "cmp qword ptr [rip + WOST_KPTI_ROUTE + {route_state}], 2",
    "je wost_kpti_stop",
    "jmp 7f",
    "15:",
    "mov r11, cr3",
    "shr r11, 12",
    "shl r11, 12",
    "cmp r11, qword ptr [rip + WOST_KPTI_ROUTE + {route_full}]",
    "jne wost_kpti_load_full_and_stop",
    "mov r10, qword ptr [rip + WOST_KPTI_ROUTE + {route_stripped}]",
    "mov rdx, rsp",
    "xor eax, eax",
    "mov rdi, qword ptr [rip + WOST_KPTI_ROUTE + {route_transition_stack}]",
    "sub rdi, {entry_stack_bytes}",
    "mov rcx, {entry_stack_words}",
    "rep stosq",
    "mov rdi, qword ptr [rip + WOST_KPTI_ROUTE + {route_interrupt_stack}]",
    "sub rdi, {entry_stack_bytes}",
    "mov rcx, {entry_stack_words}",
    "rep stosq",
    "mov rbx, qword ptr [rip + WOST_KPTI_ROUTE + {route_transition_stack}]",
    "sub rbx, {user_frame_bytes}",
    "mov rsi, rdx",
    "mov rdi, rbx",
    "mov rcx, {register_words}",
    "rep movsq",
    "lea rsi, [rdx + {frame_rip}]",
    "lea rdi, [rbx + {user_frame_rip}]",
    "mov rcx, {iret_words}",
    ".globl wost_kpti_exit_last_secret_access",
    "wost_kpti_exit_last_secret_access:",
    "rep movsq",
    "mov rsp, rbx",
    ".globl wost_kpti_exit_load_stripped_root",
    "wost_kpti_exit_load_stripped_root:",
    "mov cr3, r10",
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
    "cmp byte ptr [rip + {clear}], 0",
    "je 10f",
    "verw word ptr [rip + {verw_selector}]",
    "10:",
    ".globl wost_kpti_exit_iret",
    "wost_kpti_exit_iret:",
    "iretq",
    // Kernel and shared-root returns keep the vector and error-code words below the iret frame.
    "6:",
    "7:",
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
    // With vector and error code stepped over, the saved `CS` is at `[rsp + 8]`.
    "test byte ptr [rsp + 8], 3",
    "jz 2f",
    // `VERW` flushes CPU buffers on the way to ring 3 unless the processor reports immunity.
    "cmp byte ptr [rip + {clear}], 0",
    "je 2f",
    "sub rsp, 8",
    "mov word ptr [rsp], {kernel_data}",
    "verw word ptr [rsp]",
    "add rsp, 8",
    "2:",
    "iretq",
    // First entry builds the raw frame the interrupt path validates and joins its return tail.
    ".globl wost_leave_for_user",
    ".type wost_leave_for_user,@function",
    "wost_leave_for_user:",
    "push {user_data}",
    "push rsi",
    "push {user_entry_flags}",
    "push {user_code}",
    "push rdi",
    "push 0",
    "push 0",
    ".rept {register_words}",
    "push 0",
    ".endr",
    "jmp wost_interrupt_return",
    ".size wost_leave_for_user, . - wost_leave_for_user",
    // No Rust and no ordinary memory: restore the full root if the route names one, then stop.
    "wost_kpti_load_full_and_stop:",
    "mov r11, qword ptr [rip + WOST_KPTI_ROUTE + {route_full}]",
    "mov cr3, r11",
    "wost_kpti_stop:",
    "cli",
    // Ports only: whatever root is live, nothing outside the enclave is touched.
    ".irp byte, 0x0d,0x0a,0x5b,0x65,0x72,0x72,0x6f,0x72,0x5d,0x20,0x77,0x6f,0x73,0x74,0x3a,0x20,0x6b,0x70,0x74,0x69,0x20,0x73,0x74,0x6f,0x70,0x0d,0x0a",
    "mov ecx, {transmit_polls}",
    "16:",
    "mov dx, {com1_line_status}",
    "in al, dx",
    "test al, {transmit_empty}",
    "jnz 17f",
    "dec ecx",
    "jnz 16b",
    "17:",
    "mov dx, {com1}",
    "mov al, \\byte",
    "out dx, al",
    ".endr",
    "11:",
    "hlt",
    "jmp 11b",
    ".popsection",
    stub_bytes = const STUB_BYTES,
    raw_frame_bytes = const size_of::<RawTrapFrame>(),
    raw_frame_words = const size_of::<RawTrapFrame>() / size_of::<u64>(),
    user_frame_bytes = const 20 * size_of::<u64>(),
    register_words = const 15,
    iret_words = const 5,
    frame_rip = const core::mem::offset_of!(RawTrapFrame, instruction_pointer),
    frame_cs = const core::mem::offset_of!(RawTrapFrame, code_selector),
    frame_vector = const core::mem::offset_of!(RawTrapFrame, vector),
    double_fault = const exception::DOUBLE_FAULT as u64,
    user_frame_rip = const 15 * size_of::<u64>(),
    entry_stack_bytes = const super::kpti::STACK_BYTES,
    entry_stack_words = const super::kpti::STACK_BYTES / size_of::<u64>(),
    route_state = const super::kpti::route::STATE,
    route_full = const super::kpti::route::FULL_ROOT,
    route_stripped = const super::kpti::route::STRIPPED_ROOT,
    route_task_stack = const super::kpti::route::TASK_STACK_TOP,
    route_transition_stack = const super::kpti::route::TRANSITION_STACK_TOP,
    route_interrupt_stack = const super::kpti::route::INTERRUPT_STACK_TOP,
    clear = sym MDS_CLEAR,
    verw_selector = sym KPTI_VERW_SELECTOR,
    smap = sym SMAP_ARMED,
    com1 = const COM1,
    com1_line_status = const COM1 + 5,
    transmit_empty = const 1 << 5,
    transmit_polls = const 10_000,
    kernel_data = const (segment::KERNEL_DATA_BITS as u32),
    architectural = const exception::COUNT,
    interrupts = const idt::TABLE_LEN - exception::COUNT as usize,
    error_code_vectors = const exception::ERROR_CODE_VECTORS,
    dispatch = sym dispatch,
    user_data = const (segment::USER_DATA_BITS as u64),
    user_code = const (segment::USER_CODE_BITS as u64),
    user_entry_flags = const FLAGS_INTERRUPT | cpu::FLAGS_RESERVED,
);
