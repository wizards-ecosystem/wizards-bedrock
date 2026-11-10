use super::*;

/// Where the innermost live handler's assembly frame begins.
///
/// A number rather than a pointer: nothing safe can reach the frame through
/// it, and the machine selftest needs only to subtract.
#[doc(hidden)]
#[cfg(feature = "selftest")]
pub fn live_frame_address() -> u64 {
    LIVE_FRAME.load(Ordering::Relaxed) as u64
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

/// Whether the entry path clears `AC`.
#[cfg(feature = "selftest")]
pub fn access_clear_is_armed() -> bool {
    // SAFETY: a one-byte read of a value only ever written during boot.
    unsafe { SMAP_ARMED == 1 }
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
#[unsafe(link_section = ".kpti.state.route")]
static mut BUFFER_CLEARS: u64 = 0;

/// Whether the buffer flush is armed.
#[cfg(feature = "selftest")]
pub fn buffer_clear_is_armed() -> bool {
    // SAFETY: a one-byte read of a value only ever written during boot.
    unsafe { MDS_CLEAR == 1 }
}

/// Whether the CPU's interrupt descriptor table register points at the table
/// [`init`] built, with the limit it wrote.
#[cfg(feature = "selftest")]
pub fn table_is_loaded() -> bool {
    idt_is_loaded()
}

impl InterruptState {
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
