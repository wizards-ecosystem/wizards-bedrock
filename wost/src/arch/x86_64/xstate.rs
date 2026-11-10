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

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
use selftest::REFUSE_USER_STATE_ISOLATION;
#[cfg(feature = "selftest")]
pub use selftest::{
    Probe, read_probe, save_stays_inside_its_area, without_user_state_isolation_for_test,
    write_probe,
};

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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    let (low, high) = register_bits::u32_halves(mask);
    // SAFETY: the caller's.
    unsafe {
        asm!(
            "xsetbv",
            in("ecx") 0,
            in("eax") low,
            in("edx") high,
            options(nostack, preserves_flags),
        );
    }
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
