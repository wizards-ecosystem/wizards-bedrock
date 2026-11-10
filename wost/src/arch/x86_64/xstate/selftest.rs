use super::*;
use crate::sync::Injection;

#[cfg(feature = "selftest")]
pub(super) static REFUSE_USER_STATE_ISOLATION: Injection = Injection::new();

/// Run one check with live user-state isolation withheld.
#[cfg(feature = "selftest")]
pub fn without_user_state_isolation_for_test<T>(operation: impl FnOnce() -> T) -> T {
    REFUSE_USER_STATE_ISOLATION.with(true, operation)
}

/// Whether a save leaves a sentinel immediately beyond [`bytes`] untouched.
///
/// Verifies the independently published save form and size agree; `false` when extended state
/// is disabled.
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
    // `XSAVE` writes only the requested components and the header, so only the sentinel is read.
    // SAFETY: `area` is 64-byte aligned by its `repr` and at least `bytes` long, checked above,
    // which is exactly `save`'s contract.
    unsafe { save(area.0.as_mut_ptr()) };

    area.0[bytes..bytes + SENTINEL]
        .iter()
        .all(|byte| *byte == MARK)
}

/// Values spanning every extended-state component this kernel enables.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg(feature = "selftest")]
pub struct Probe {
    /// One exactly representable value on the x87 register stack.
    pub x87: i64,
    /// The low quadword of `xmm0`, in the legacy SSE component.
    pub xmm: u64,
    /// The low quadword of `ymm0`'s upper half, in the AVX component.
    pub ymm: u64,
}

/// Put `probe` in x87, SSE and AVX state, refusing unless all three components are live.
///
/// The soft-float target cannot keep values in these registers, so this test-only clobber
/// cannot collide with compiled code.
#[cfg(feature = "selftest")]
pub fn write_probe(probe: Probe) -> bool {
    if !configured_policy_is_live() {
        return false;
    }
    let x87 = probe.x87;
    // SAFETY: the instructions touch only the three components above, which compiled code cannot
    // hold; the mask check proves each is enabled, and `x87` stays live for the memory read.
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
/// The x87 value is popped so repeated probes cannot fill its eight-entry stack; the SSE and AVX
/// lanes are only read.
#[cfg(feature = "selftest")]
pub fn read_probe() -> Option<Probe> {
    if !configured_policy_is_live() {
        return None;
    }
    let mut x87 = 0i64;
    let xmm: u64;
    let ymm: u64;
    // SAFETY: as in `write_probe`; `x87` is writable for the instruction, and `out` registers
    // cannot overlap its pointer before the final store.
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
