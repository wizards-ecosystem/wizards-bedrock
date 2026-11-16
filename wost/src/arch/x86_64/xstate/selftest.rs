use super::*;

pub fn save_stays_inside_its_area() -> bool {
    const ROOM: usize = 4096;
    const SENTINEL: usize = 128;
    const MARK: u8 = 0xC3;

    #[repr(C, align(64))]
    struct Area([u8; ROOM]);

    if !user_state_is_isolated() {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Probe {
    pub x87: i64,
    pub xmm: u64,
    pub ymm: u64,
}

pub fn write_probe(probe: Probe) -> bool {
    if !probe_components_are_live() {
        return false;
    }
    let x87 = probe.x87;
    // SAFETY: the instructions touch only x87, SSE and AVX state, which soft-float compiled code
    // cannot hold; the mask check proves each is enabled, and `x87` stays live for the read.
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

// The x87 value is popped, so repeated probes cannot fill its eight-entry stack.
pub fn read_probe() -> Option<Probe> {
    if !probe_components_are_live() {
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

fn probe_components_are_live() -> bool {
    user_state_is_isolated() && xstate::includes_avx(MASK.load(Ordering::Acquire))
}
