//! Extended processor state: what a thread's floating-point registers are
//! saved into, and when.
//!
//! **Eager, never lazy** (D-82). Lazy save/restore is CVE-2018-3665, and it
//! buys a saving this kernel has no way to measure. Every switch saves and
//! restores, and the cost is one pair of instructions on a path that already
//! writes `CR3` and `RSP0`.
//!
//! The arithmetic — which components are present, how large each is, where it
//! sits in a compacted area — is `x86_cpuid::xstate`'s and is host-tested
//! against synthetic CPUID snapshots including the APX case that broke Linux's
//! monotonicity assumption. What is here is the four instructions and the two
//! control registers.
//!
//! Nothing is enabled until every refusal has been made: a failure path that
//! left SSE legal at CPL 3 with no save in the switch would be worse than
//! never calling [`init`] at all (D-115).
//!
//! `XCR0` is x87, SSE and AVX and nothing else (D-82). A wider mask is a
//! larger buffer for every thread and state this kernel has no user for; the
//! bits that would earn their room — CET's shadow-stack pointers — arrive with
//! CET.

use core::arch::asm;
use core::arch::x86_64::__cpuid_count;
use core::sync::atomic::{AtomicUsize, Ordering};

use x86_cpuid::xstate::{self, Component, Layout};

use super::cpu;

/// The leaf that describes extended state.
const XSTATE_LEAF: u32 = 0x0D;

/// `CPUID.01H:ECX` bit 26, the processor's own `XSAVE` support.
const XSAVE_SUPPORTED: u32 = 1 << 26;

/// `CPUID.0DH.1:EAX` bit 1, the compacted save instruction.
const XSAVEC_SUPPORTED: u32 = 1 << 1;

/// Room for a save area, in bytes.
///
/// A ceiling, not the size: the size comes from `CPUID` at boot and is what the
/// instructions are given. This is what `.bss` reserves per thread, and
/// [`init`] refuses rather than overrunning it — 576 bytes of legacy area and
/// header plus 256 for AVX leaves generous room, and a mask this kernel does
/// not set cannot make it larger.
pub const CEILING: usize = 1024;

/// Alignment `XSAVE` requires of its area.
pub const ALIGNMENT: usize = 64;

/// Bytes the save instructions actually write, or zero before [`init`].
static BYTES: AtomicUsize = AtomicUsize::new(0);

/// Which save instruction to use: 0 none, 1 `XSAVE`, 2 `XSAVEC`.
static FORM: AtomicUsize = AtomicUsize::new(0);

/// What [`init`] decided, for the boot report and the checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extended {
    /// Bytes a save area needs.
    pub bytes: usize,
    /// The mask in `XCR0`.
    pub mask: u64,
    /// Whether the compacted form is in use.
    pub compacted: bool,
}

/// Turn on extended-state saving, and report what it costs.
///
/// `None` where the processor has no `XSAVE` at all, in which case nothing is
/// saved and nothing needs to be: without `XSAVE` there is no `XCR0`, the
/// legacy x87 and SSE state is all there is, and `+soft-float` means the
/// kernel never touches it — so a thread's registers survive a switch by not
/// being disturbed.
///
/// Safe: every bit written is one `CPUID` reported, and the worst an
/// unexpected one could do is fault, which is a trap the kernel reports.
pub fn init() -> Option<Extended> {
    let basic = __cpuid_count(1, 0);
    if basic.ecx & XSAVE_SUPPORTED == 0 {
        return None;
    }

    // **Every refusal happens before anything is enabled.** `CPUID.0DH`
    // reports what the *processor* supports and needs no control bit; only
    // the instructions do. The first version enabled first and asked
    // afterwards, so three failure paths left `CR0.EM` clear and
    // `CR4.OSFXSR` set — SSE legal at CPL 3 — with `FORM` at zero, which
    // means the switch saves nothing. Two programs would then share the
    // register file, and the machine would be strictly worse off than if
    // this function had never been called (D-115).
    let supported = __cpuid_count(XSTATE_LEAF, 0);
    let available = u64::from(supported.eax) | (u64::from(supported.edx) << 32);
    let mask = available & xstate::PERMITTED;
    // x87 is mandatory in `XCR0`; a processor reporting otherwise is one this
    // kernel does not understand, and writing the register would fault.
    if mask & xstate::X87 == 0 {
        return None;
    }

    // Sub-leaf 1 reports which save forms exist.
    let forms = __cpuid_count(XSTATE_LEAF, 1);
    let compacted = forms.eax & XSAVEC_SUPPORTED != 0;

    let layout = Layout::compacted(mask, describe);
    if !layout.is_consistent() {
        return None;
    }
    // The uncompacted form writes every component at its architectural offset,
    // which is never below the compacted one; `CPUID.0DH.0:EBX` is that size
    // for the mask this is about to put in `XCR0`. Taking the larger means one
    // ceiling serves whichever form the processor turns out to have.
    let bytes = layout.size().max(supported.ebx as usize);
    if !(xstate::HEADER_BYTES..=CEILING).contains(&bytes) {
        return None;
    }

    // Nothing below can refuse, so from here the machine is committed.
    cpu::enable_extended_state();
    // SAFETY: every bit of `mask` was reported available by the leaf above,
    // and `CR4.OSXSAVE` is set by the line above, which is `XSETBV`'s only
    // precondition.
    unsafe { set_extended_control(mask) };

    BYTES.store(bytes, Ordering::Release);
    FORM.store(if compacted { 2 } else { 1 }, Ordering::Release);
    Some(Extended {
        bytes,
        mask,
        compacted,
    })
}

/// What one component of the state costs and whether it wants alignment.
fn describe(index: u32) -> Component {
    // A sub-leaf the processor does not describe answers with zeros, which
    // `Component::new` reads as absent.
    let leaf = __cpuid_count(XSTATE_LEAF, index);
    Component::new(leaf.eax, leaf.ecx & (1 << 1) != 0)
}

/// Bytes a save area needs, or zero if nothing is saved.
pub fn bytes() -> usize {
    BYTES.load(Ordering::Acquire)
}

/// Whether extended state is being saved at all.
pub fn is_enabled() -> bool {
    FORM.load(Ordering::Acquire) != 0
}

/// Save this CPU's extended state into `area`.
///
/// # Safety
///
/// `area` must be [`ALIGNMENT`]-aligned and at least [`bytes`] long, and
/// writable for that whole span. Nothing else may be reading or writing it.
pub unsafe fn save(area: *mut u8) {
    match FORM.load(Ordering::Acquire) {
        2 => {
            // SAFETY: the caller's, plus `XSAVEC` needs only `CR4.OSXSAVE`,
            // which `init` set before storing a non-zero form.
            unsafe {
                asm!(
                    "xsavec [{area}]",
                    area = in(reg) area,
                    in("eax") u32::MAX,
                    in("edx") u32::MAX,
                    options(nostack, preserves_flags),
                );
            }
        }
        1 => {
            // SAFETY: as above, for the uncompacted form.
            unsafe {
                asm!(
                    "xsave [{area}]",
                    area = in(reg) area,
                    in("eax") u32::MAX,
                    in("edx") u32::MAX,
                    options(nostack, preserves_flags),
                );
            }
        }
        _ => {}
    }
}

/// Restore this CPU's extended state from `area`.
///
/// # Safety
///
/// `area` must satisfy [`save`]'s requirements and must hold a state [`save`]
/// wrote. `XRSTOR` reads the header's `XCOMP_BV` to tell the two forms apart,
/// so one restore serves both.
pub unsafe fn restore(area: *const u8) {
    if FORM.load(Ordering::Acquire) == 0 {
        return;
    }
    // SAFETY: the caller's, plus `CR4.OSXSAVE` is set — `init` stored a
    // non-zero form only after setting it.
    unsafe {
        asm!(
            "xrstor [{area}]",
            area = in(reg) area,
            in("eax") u32::MAX,
            in("edx") u32::MAX,
            options(nostack, preserves_flags),
        );
    }
}

/// Write `mask` to `XCR0`.
///
/// # Safety
///
/// `CR4.OSXSAVE` must be set, and every bit of `mask` must be one
/// `CPUID.0DH.0` reported as available with bit 0 among them — `XSETBV` faults
/// otherwise.
unsafe fn set_extended_control(mask: u64) {
    // SAFETY: the caller's.
    unsafe {
        asm!(
            "xsetbv",
            in("ecx") 0,
            in("eax") mask as u32,
            in("edx") (mask >> 32) as u32,
            options(nostack, preserves_flags),
        );
    }
}

/// Put `value` in the low quadword of `xmm0`.
///
/// The only thing in this kernel that touches a vector register, and it exists
/// so a check can ask whether a switch preserved one.
///
/// **The clobber is not declared, and cannot be.** `x86_64-unknown-none` is
/// built `-sse,+soft-float`, so `xmm0` is not a register LLVM knows how to
/// allocate — naming it in a clobber list is rejected. What makes this sound
/// is therefore the *target feature* and not the asm contract: no compiled
/// code in this kernel holds anything in a vector register, which is also what
/// lets the switch's save sit before the switch (D-108). Written down because
/// soundness resting on a build flag should be visible where the flag is
/// relied on (D-115).
pub fn write_vector_probe(value: u64) {
    // SAFETY: writes one register no compiled code in this kernel holds, per
    // the note above, and reads nothing else.
    unsafe {
        asm!(
            "movq xmm0, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Read back what [`write_vector_probe`] put there.
pub fn read_vector_probe() -> u64 {
    let value: u64;
    // SAFETY: as above, reading rather than writing.
    unsafe {
        asm!(
            "movq {value}, xmm0",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

/// Read `XCR0` back, for the check that the write took.
///
/// Zero before [`init`] succeeds, which is also what a processor without
/// `XSAVE` reports — the two are the same answer for the same reason.
pub fn extended_control() -> u64 {
    // Guarded on the *control register*, not on `FORM`. Reading the Rust flag
    // here made the check that asks about `XCR0` return a constant zero on the
    // path where it mattered — D-105's `VERW`-arming lesson, in a fresh
    // instance found by D-115.
    if !cpu::extended_state_is_enabled() {
        return 0;
    }
    let (low, high): (u32, u32);
    // SAFETY: `XGETBV` needs `CR4.OSXSAVE`, which is set whenever a form is
    // stored, and reads a register with no side effects.
    unsafe {
        asm!(
            "xgetbv",
            in("ecx") 0,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    u64::from(low) | (u64::from(high) << 32)
}
