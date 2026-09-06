//! Eager x87, SSE, and AVX state isolation (D-82).
//!
//! `x86_cpuid::xstate` owns the host-tested layout arithmetic; this module owns
//! the control registers and save instructions. All fallible validation occurs
//! before enabling user-visible state (D-115). Lazy switching is excluded by
//! the CVE-2018-3665 class of register-state disclosure.

use core::arch::asm;
use core::arch::x86_64::__cpuid_count;
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicBool;
use core::sync::atomic::{AtomicUsize, Ordering};

use x86_cpuid::xstate::{self, Alignment, Component, Layout};

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
/// `None` where the processor has no usable `XSAVE`; user launch then remains
/// closed because thread switches cannot isolate that register state.
///
/// Safe: state bits and their instruction features are checked before any
/// control register is changed.
pub fn init() -> Option<Extended> {
    let basic = __cpuid_count(1, 0);
    if basic.ecx & XSAVE_SUPPORTED == 0 {
        return None;
    }

    // Preflight before enabling anything, so a refusal cannot leave SSE usable
    // while the context switch still saves no extended state (D-115).
    let supported = __cpuid_count(XSTATE_LEAF, 0);
    let available = u64::from(supported.eax) | (u64::from(supported.edx) << 32);
    if !xstate::has_permitted_components(available) {
        return None;
    }
    let mask = xstate::PERMITTED;
    if !xstate::avx_state_has_instruction_support(mask, basic.ecx) {
        return None;
    }
    // x87 is mandatory in `XCR0`; a processor reporting otherwise is one this
    // kernel does not understand, and writing the register would fault.
    if mask & xstate::X87 == 0 {
        return None;
    }

    let forms = __cpuid_count(XSTATE_LEAF, 1);
    let compacted = forms.eax & XSAVEC_SUPPORTED != 0;

    let layout = Layout::compacted(mask, describe);
    if !layout.is_consistent() {
        return None;
    }
    let avx = __cpuid_count(XSTATE_LEAF, 2);
    let standard_bytes = xstate::standard_component_extent(
        Component::new(
            avx.eax,
            if avx.ecx & (1 << 1) != 0 {
                Alignment::Bytes64
            } else {
                Alignment::Natural
            },
        ),
        avx.ebx,
    )?;
    let bytes = if compacted {
        layout.size()
    } else {
        standard_bytes
    };
    if !(xstate::HEADER_BYTES..=CEILING).contains(&bytes) {
        return None;
    }

    // Every fallible preflight precedes the irreversible control-register write.
    cpu::enable_extended_state();
    // SAFETY: every bit was reported available, the layout checked component
    // dependencies, the feature gate checked AVX, and `CR4.OSXSAVE` is set.
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
    Component::new(
        leaf.eax,
        if leaf.ecx & (1 << 1) != 0 {
            Alignment::Bytes64
        } else {
            Alignment::Natural
        },
    )
}

/// Bytes a save area needs, or zero if nothing is saved.
pub fn bytes() -> usize {
    BYTES.load(Ordering::Acquire)
}

/// Whether extended state is being saved at all.
pub fn is_enabled() -> bool {
    FORM.load(Ordering::Acquire) != 0
}

/// Whether the configured save machinery and hardware still isolate user state.
pub fn user_state_is_isolated() -> bool {
    let isolated = configured_policy_is_live();
    #[cfg(feature = "selftest")]
    let isolated = isolated && !REFUSE_USER_STATE_ISOLATION.load(Ordering::Acquire);
    isolated
}

fn configured_policy_is_live() -> bool {
    is_enabled()
        && cpu::extended_state_is_enabled()
        && xstate::matches_permitted_configuration(extended_control())
}

#[cfg(feature = "selftest")]
static REFUSE_USER_STATE_ISOLATION: AtomicBool = AtomicBool::new(false);

/// Run one check with live user-state isolation withheld.
#[cfg(feature = "selftest")]
pub fn without_user_state_isolation_for_test<T>(operation: impl FnOnce() -> T) -> T {
    let previous = REFUSE_USER_STATE_ISOLATION.swap(true, Ordering::AcqRel);
    let result = operation();
    REFUSE_USER_STATE_ISOLATION.store(previous, Ordering::Release);
    result
}

/// Whether a save leaves a sentinel immediately beyond [`bytes`] untouched.
///
/// This verifies the independently published save form and size agree (D-144).
/// Returns `false` when extended state is disabled.
#[cfg(feature = "selftest")]
pub fn save_stays_inside_its_area() -> bool {
    const ROOM: usize = 4096;
    const SENTINEL: usize = 128;
    const MARK: u8 = 0xC3;

    #[repr(C, align(64))]
    struct Area([u8; ROOM]);

    if !configured_policy_is_live() {
        return false;
    }
    let bytes = bytes();
    if bytes == 0 || bytes + SENTINEL > ROOM {
        return false;
    }

    let mut area = Area([MARK; ROOM]);
    // `XSAVE` writes only the requested components and the header, so the
    // sentinel beyond them is the only part read back.
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
///
/// # Panics
///
/// Panics if the live control registers no longer match the sized policy.
pub(super) unsafe fn save(area: *mut u8) {
    assert!(
        configured_policy_is_live(),
        "extended-state controls diverged from the sized save policy"
    );
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
/// `area` must satisfy [`save`]'s requirements and hold either a state [`save`]
/// wrote or [`bytes`] fully zeroed bytes. A zero `XSTATE_BV` and `XCOMP_BV`
/// encode architectural initial state in standard form; zero reserved header
/// bytes satisfy `XRSTOR`'s format checks.
///
/// # Panics
///
/// Panics if the live control registers no longer match the sized policy.
pub(super) unsafe fn restore(area: *const u8) {
    if FORM.load(Ordering::Acquire) == 0 {
        return;
    }
    assert!(
        configured_policy_is_live(),
        "extended-state controls diverged from the sized restore policy"
    );
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
#[cfg(feature = "selftest")]
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
/// The soft-float target cannot keep values in these registers, which is why
/// this test-only clobber cannot collide with compiled code (D-108, D-228).
/// It refuses unless all three components are live.
#[cfg(feature = "selftest")]
pub fn write_probe(probe: Probe) -> bool {
    if !configured_policy_is_live() {
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
#[cfg(feature = "selftest")]
pub fn read_probe() -> Option<Probe> {
    if !configured_policy_is_live() {
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
    // `XGETBV` legality comes from live `CR4.OSXSAVE`, not the later `FORM`
    // publication; consulting `FORM` here would make the read self-referential.
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
