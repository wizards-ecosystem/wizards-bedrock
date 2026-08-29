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

/// Bytes the save instructions actually write, or zero before [`init`].
static BYTES: AtomicUsize = AtomicUsize::new(0);

/// Which save instruction to use: 0 none, 1 `XSAVE`, 2 `XSAVEC`.
static FORM: AtomicUsize = AtomicUsize::new(0);

/// Components the save and restore instructions are asked to transfer.
const OPERATION_MASK: u32 = u32::MAX;

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

/// Whether a save writes only inside the area [`bytes`] asks for.
///
/// The hazard this exists for: `bytes` is computed for whichever form `init`
/// chose, and [`save`] dispatches on a separate word. They are stored together
/// and agree — and nothing checked that they still do. A `FORM` naming the
/// uncompacted path with a compacted `bytes` writes **past the end of every
/// save area on the machine**, which is a `ThreadContext`-sized buffer inside
/// a thread's slot.
///
/// Checked by consequence rather than by label: the area is followed by a
/// sentinel, and the save must not touch it. That catches the disagreement
/// whichever direction it goes, and would catch a `bytes` that was simply too
/// small for reasons nobody has thought of yet (D-144).
///
/// `false` where extended state is off, since there is nothing to save.
pub fn save_stays_inside_its_area() -> bool {
    /// Generous beside any `XCR0` this kernel enables — x87, SSE and AVX come
    /// to well under a kilobyte — and the check refuses rather than overflows
    /// if that ever stops being true.
    const ROOM: usize = 4096;
    /// Bytes of sentinel past the area, and the pattern written into them.
    const SENTINEL: usize = 128;
    const MARK: u8 = 0xC3;

    #[repr(C, align(64))]
    struct Area([u8; ROOM]);

    if !is_enabled() {
        return false;
    }
    let bytes = bytes();
    if bytes == 0 || bytes + SENTINEL > ROOM {
        return false;
    }

    let mut area = Area([MARK; ROOM]);
    // `XSAVE` writes only the components whose bits it is given and the header;
    // the rest of the area is left as it was, so the sentinel is the only part
    // this reads back.
    // SAFETY: `area` is 64-byte aligned by its `repr` and at least `bytes`
    // long, checked above — which is exactly `save`'s contract.
    unsafe { save(area.0.as_mut_ptr()) };

    area.0[bytes..bytes + SENTINEL]
        .iter()
        .all(|byte| *byte == MARK)
}

/// Save this CPU's extended state into `area`.
///
/// # Safety
///
/// `area` must be 64-byte aligned and at least [`bytes`] long, and
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
                    in("eax") OPERATION_MASK,
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
                    in("eax") OPERATION_MASK,
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
            in("eax") OPERATION_MASK,
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

/// Values spanning every extended-state component this kernel enables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// One exactly representable value on the x87 register stack.
    pub x87: i64,
    /// The low quadword of `xmm0`, in the legacy SSE component.
    pub xmm: u64,
    /// The low quadword of `ymm0`'s upper half, in the AVX component.
    pub ymm: u64,
}

/// Put `probe` in x87, SSE and AVX state.
///
/// These are the only floating-point/vector instructions in the kernel. The
/// clobbers cannot be declared because `x86_64-unknown-none` is built
/// `-sse,+soft-float`; no compiled code can hold these registers, which is
/// also what lets the switch save before changing stacks (D-108, D-228).
///
/// Returns `false` unless all three components are live. `CR4.OSFXSR`,
/// `CR4.OSXSAVE` and `CR0.EM` have their required values whenever [`init`]
/// publishes that mask, so safe callers cannot reach an undefined instruction.
pub fn write_probe(probe: Probe) -> bool {
    if !is_enabled() || extended_control() & xstate::PERMITTED != xstate::PERMITTED {
        return false;
    }
    let x87 = probe.x87;
    // SAFETY: the instructions touch only the three components named above,
    // which compiled code cannot hold. The mask check proves each instruction
    // is enabled, and `x87` remains live for the memory read.
    unsafe {
        asm!(
            "fninit\nfild qword ptr [{x87}]",
            "movq xmm0, {xmm}",
            "vmovq xmm1, {ymm}",
            "vinsertf128 ymm0, ymm0, xmm1, 1",
            x87 = in(reg) &x87,
            xmm = in(reg) probe.xmm,
            ymm = in(reg) probe.ymm,
            options(nostack, preserves_flags),
        );
    }
    true
}

/// Read and consume the values [`write_probe`] installed.
///
/// The x87 value is popped so repeated probes cannot fill its eight-entry
/// stack. The SSE and AVX lanes are only read.
pub fn read_probe() -> Option<Probe> {
    if !is_enabled() || extended_control() & xstate::PERMITTED != xstate::PERMITTED {
        return None;
    }
    let mut x87 = 0i64;
    let xmm: u64;
    let ymm: u64;
    // SAFETY: as in `write_probe`; `x87` is writable for the instruction and
    // `out` registers cannot overlap its pointer before the final store.
    unsafe {
        asm!(
            "movq {xmm}, xmm0",
            "vextractf128 xmm1, ymm0, 1",
            "vmovq {ymm}, xmm1",
            "fistp qword ptr [{x87}]",
            x87 = in(reg) &mut x87,
            xmm = out(reg) xmm,
            ymm = out(reg) ymm,
            options(nostack, preserves_flags),
        );
    }
    Some(Probe { x87, xmm, ymm })
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
