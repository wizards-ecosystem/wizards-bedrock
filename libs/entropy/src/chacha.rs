use crate::{Secret, Wipe, set_word, word};

const SIGMA: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

fn quarter_round(state: &mut [u8; 64], a: usize, b: usize, c: usize, d: usize) {
    let [mut va, mut vb, mut vc, mut vd] = [a, b, c, d].map(|at| word(state, at));
    va = va.wrapping_add(vb);
    vd = (vd ^ va).rotate_left(16);
    vc = vc.wrapping_add(vd);
    vb = (vb ^ vc).rotate_left(12);
    va = va.wrapping_add(vb);
    vd = (vd ^ va).rotate_left(8);
    vc = vc.wrapping_add(vd);
    vb = (vb ^ vc).rotate_left(7);
    for (at, value) in [(a, va), (b, vb), (c, vc), (d, vd)] {
        set_word(state, at, value);
    }
}

fn input(key: &[u8; 32], counter: u32, nonce: &[u8; 12], at: usize) -> u32 {
    match at {
        0..4 => SIGMA[at],
        4..12 => word(key, at - 4),
        12 => counter,
        _ => word(nonce, at - 13),
    }
}

// The rounds run in `out`, so no copy of the key's state outlives the caller's own wipe.
pub(crate) fn block(key: &[u8; 32], counter: u32, nonce: &[u8; 12], out: &mut [u8; 64]) {
    for at in 0..16 {
        set_word(out, at, input(key, counter, nonce, at));
    }
    for _ in 0..10 {
        quarter_round(out, 0, 4, 8, 12);
        quarter_round(out, 1, 5, 9, 13);
        quarter_round(out, 2, 6, 10, 14);
        quarter_round(out, 3, 7, 11, 15);
        quarter_round(out, 0, 5, 10, 15);
        quarter_round(out, 1, 6, 11, 12);
        quarter_round(out, 2, 7, 8, 13);
        quarter_round(out, 3, 4, 9, 14);
    }
    for at in 0..16 {
        let mixed = word(out, at).wrapping_add(input(key, counter, nonce, at));
        set_word(out, at, mixed);
    }
}

pub(crate) fn keystream(key: &[u8; 32], out: &mut [u8], wipe: Wipe) {
    let mut scratch = Secret::<64>::zero(wipe);
    let mut counter = 0u32;
    for piece in out.chunks_mut(64) {
        block(key, counter, &[0; 12], &mut scratch.bytes);
        piece.copy_from_slice(&scratch.bytes[..piece.len()]);
        counter = counter.wrapping_add(1);
    }
}
