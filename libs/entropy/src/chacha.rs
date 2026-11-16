use crate::{Secret, Wipe};

const SIGMA: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(7);
}

pub(crate) fn block(key: &[u8; 32], counter: u32, nonce: &[u8; 12], out: &mut [u8; 64]) {
    let mut input = [0u32; 16];
    input[..4].copy_from_slice(&SIGMA);
    for (word, bytes) in input[4..12].iter_mut().zip(key.chunks_exact(4)) {
        *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    input[12] = counter;
    for (word, bytes) in input[13..].iter_mut().zip(nonce.chunks_exact(4)) {
        *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    let mut state = input;
    for _ in 0..10 {
        quarter_round(&mut state, 0, 4, 8, 12);
        quarter_round(&mut state, 1, 5, 9, 13);
        quarter_round(&mut state, 2, 6, 10, 14);
        quarter_round(&mut state, 3, 7, 11, 15);
        quarter_round(&mut state, 0, 5, 10, 15);
        quarter_round(&mut state, 1, 6, 11, 12);
        quarter_round(&mut state, 2, 7, 8, 13);
        quarter_round(&mut state, 3, 4, 9, 14);
    }
    for ((bytes, word), initial) in out.chunks_exact_mut(4).zip(state).zip(input) {
        bytes.copy_from_slice(&word.wrapping_add(initial).to_le_bytes());
    }
    input.fill(0);
    state.fill(0);
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
