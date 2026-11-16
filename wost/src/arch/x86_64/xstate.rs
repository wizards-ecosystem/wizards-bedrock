use core::arch::asm;
use core::arch::x86_64::__cpuid_count;
use core::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};

use x86_cpuid::xstate::{self, Alignment, Component, Layout};

use super::cpu;

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::{Probe, read_probe, save_stays_inside_its_area, write_probe};

const XSTATE_LEAF: u32 = 0x0D;

const XSAVE_SUPPORTED: u32 = 1 << 26;

const XSAVEC_SUPPORTED: u32 = 1 << 1;

const AVX_INDEX: u32 = 2;

// `CPUID.0DH.i:ECX` bit 1: the component starts on a 64-byte boundary in the compacted form.
const ALIGNED_TO_64: u32 = 1 << 1;

pub(crate) const CEILING: usize = 1024;

static BYTES: AtomicUsize = AtomicUsize::new(0);

static FORM: AtomicU8 = AtomicU8::new(SaveForm::None as u8);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum SaveForm {
    None,
    Standard,
    Compacted,
}

impl SaveForm {
    fn current() -> Self {
        match FORM.load(Ordering::Acquire) {
            1 => Self::Standard,
            2 => Self::Compacted,
            _ => Self::None,
        }
    }
}

static MASK: AtomicU64 = AtomicU64::new(0);

const OPERATION_MASK: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Extended {
    pub bytes: usize,
    pub mask: u64,
    pub compacted: bool,
}

pub fn init() -> Option<Extended> {
    let basic = __cpuid_count(1, 0);
    if basic.ecx & XSAVE_SUPPORTED == 0 {
        return None;
    }

    // Preflight first, so a refusal cannot leave SSE usable while switches save no extended state.
    let supported = __cpuid_count(XSTATE_LEAF, 0);
    let available = u64::from(supported.eax) | (u64::from(supported.edx) << 32);
    let mask = xstate::enabled_components(available, basic.ecx)?;

    let forms = __cpuid_count(XSTATE_LEAF, 1);
    let compacted = forms.eax & XSAVEC_SUPPORTED != 0;

    let layout = Layout::compacted(mask, describe);
    if !layout.is_consistent() {
        return None;
    }
    let bytes = if compacted {
        layout.size()
    } else if xstate::includes_avx(mask) {
        xstate::standard_component_extent(
            describe(AVX_INDEX),
            __cpuid_count(XSTATE_LEAF, AVX_INDEX).ebx,
        )?
    } else {
        xstate::HEADER_BYTES
    };
    if !(xstate::HEADER_BYTES..=CEILING).contains(&bytes) {
        return None;
    }

    cpu::enable_extended_state();
    // SAFETY: every bit was reported available, x87 and SSE are among them, AVX only with its
    // instruction support, and `CR4.OSXSAVE` is set.
    unsafe { set_extended_control(mask) };

    MASK.store(mask, Ordering::Release);
    BYTES.store(bytes, Ordering::Release);
    let form = if compacted {
        SaveForm::Compacted
    } else {
        SaveForm::Standard
    };
    FORM.store(form as u8, Ordering::Release);
    Some(Extended {
        bytes,
        mask,
        compacted,
    })
}

fn describe(index: u32) -> Component {
    // A sub-leaf the processor does not describe answers zeros, which reads as absent.
    let leaf = __cpuid_count(XSTATE_LEAF, index);
    Component::new(
        leaf.eax,
        if leaf.ecx & ALIGNED_TO_64 != 0 {
            Alignment::Bytes64
        } else {
            Alignment::Natural
        },
    )
}

pub fn bytes() -> usize {
    BYTES.load(Ordering::Acquire)
}

pub fn is_enabled() -> bool {
    SaveForm::current() != SaveForm::None
}

pub fn user_state_is_isolated() -> bool {
    is_enabled()
        && cpu::extended_state_is_enabled()
        && extended_control() == MASK.load(Ordering::Acquire)
}

/// # Safety
///
/// `area` must be 64-byte aligned, at least [`bytes`] long and writable for that whole span.
/// Nothing else may be reading or writing it.
pub(super) unsafe fn save(area: *mut u8) {
    assert!(
        user_state_is_isolated(),
        "extended-state controls diverged from the sized save policy"
    );
    match SaveForm::current() {
        SaveForm::Compacted => {
            // SAFETY: the caller supplies an exclusive, 64-byte-aligned area of at least
            // `bytes()`, all `XSAVEC` writes for the mask `init` sized, and the assert above
            // re-read `CR4.OSXSAVE` and `XCR0`.
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
        SaveForm::Standard => {
            // SAFETY: as for `XSAVEC`: `init` sized `bytes()` for the standard form in this case.
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
        SaveForm::None => {}
    }
}

/// # Safety
///
/// `area` must satisfy [`save`]'s requirements and hold either a state [`save`] wrote or one
/// [`write_initial`] wrote.
pub(super) unsafe fn restore(area: *const u8) {
    if SaveForm::current() == SaveForm::None {
        return;
    }
    assert!(
        user_state_is_isolated(),
        "extended-state controls diverged from the sized restore policy"
    );
    // SAFETY: the caller supplies an exclusive, aligned area holding a `save` or `write_initial`
    // image, so `XRSTOR` reads only well-formed state, and the assert above re-read the controls.
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

/// # Safety
///
/// `area` must be writable for [`CEILING`] bytes, with nothing else reading or writing it.
pub(super) unsafe fn write_initial(area: *mut u8) {
    const MXCSR_OFFSET: usize = 24;
    const MXCSR_RESET: u32 = 0x1F80;
    // SAFETY: the caller's span covers `CEILING` bytes.
    unsafe { area.write_bytes(0, CEILING) };
    // `XRSTOR` loads `MXCSR` from memory even with `XSTATE_BV` zero, so it gets the reset value.
    // SAFETY: the legacy region lies inside that span, and the write needs no alignment.
    unsafe {
        area.wrapping_add(MXCSR_OFFSET)
            .cast::<u32>()
            .write_unaligned(MXCSR_RESET);
    }
}

/// # Safety
///
/// `CR4.OSXSAVE` must be set, and every bit of `mask` must be one `CPUID.0DH.0` reported as
/// available, with bit 0 among them; `XSETBV` faults otherwise.
unsafe fn set_extended_control(mask: u64) {
    let (low, high) = register_bits::u32_halves(mask);
    // SAFETY: the caller guarantees `CR4.OSXSAVE` is set and that `mask` holds only reported
    // components, x87 among them, which is everything `XSETBV` checks.
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

pub fn extended_control() -> u64 {
    // Gated on live `CR4.OSXSAVE`, not on `FORM`, which `init` publishes only afterwards.
    if !cpu::extended_state_is_enabled() {
        return 0;
    }
    let (low, high): (u32, u32);
    // SAFETY: `XGETBV` needs `CR4.OSXSAVE`, which is set whenever a form is stored, and reads a
    // register with no side effects.
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
