use crate::Wipe;

const BLOCK: usize = 64;

const IV: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const SCHEDULE: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

// The chaining value is kept as little-endian bytes, so the wipe hook can clear it.
pub(crate) struct Blake2s {
    state: [u8; 32],
    counter: u64,
    pub(crate) buffer: [u8; BLOCK],
    filled: usize,
    wipe: Wipe,
}

impl Drop for Blake2s {
    fn drop(&mut self) {
        (self.wipe)(&mut self.state);
        (self.wipe)(&mut self.buffer);
    }
}

fn mix(v: &mut [u32; 16], lanes: [usize; 4], x: u32, y: u32) {
    let [a, b, c, d] = lanes;
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(12);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(8);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(7);
}

impl Blake2s {
    pub(crate) fn new(wipe: Wipe) -> Self {
        Self::with_key(&[], wipe)
    }

    pub(crate) fn keyed(key: &[u8; 32], wipe: Wipe) -> Self {
        Self::with_key(key, wipe)
    }

    fn with_key(key: &[u8], wipe: Wipe) -> Self {
        let mut words = IV;
        words[0] ^= 0x0101_0000 ^ ((key.len() as u32) << 8) ^ 32;
        let mut hash = Self {
            state: [0; 32],
            counter: 0,
            buffer: [0; BLOCK],
            filled: if key.is_empty() { 0 } else { BLOCK },
            wipe,
        };
        hash.store_state(&words);
        hash.buffer[..key.len()].copy_from_slice(key);
        hash
    }

    fn store_state(&mut self, words: &[u32; 8]) {
        for (bytes, word) in self.state.chunks_exact_mut(4).zip(words) {
            bytes.copy_from_slice(&word.to_le_bytes());
        }
    }

    pub(crate) fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            if self.filled == BLOCK {
                self.counter = self.counter.wrapping_add(BLOCK as u64);
                self.compress(false);
                self.filled = 0;
            }
            let (head, rest) = data.split_at((BLOCK - self.filled).min(data.len()));
            self.buffer[self.filled..self.filled + head.len()].copy_from_slice(head);
            self.filled += head.len();
            data = rest;
        }
    }

    pub(crate) fn finalize_into(mut self, out: &mut [u8; 32]) {
        self.counter = self.counter.wrapping_add(self.filled as u64);
        self.buffer[self.filled..].fill(0);
        self.compress(true);
        out.copy_from_slice(&self.state);
    }

    fn compress(&mut self, last: bool) {
        let mut m = [0u32; 16];
        for (word, bytes) in m.iter_mut().zip(self.buffer.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let mut v = [0u32; 16];
        for (word, bytes) in v[..8].iter_mut().zip(self.state.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        v[8..].copy_from_slice(&IV);
        v[12] ^= self.counter as u32;
        v[13] ^= (self.counter >> 32) as u32;
        if last {
            v[14] = !v[14];
        }
        for s in SCHEDULE {
            mix(&mut v, [0, 4, 8, 12], m[s[0]], m[s[1]]);
            mix(&mut v, [1, 5, 9, 13], m[s[2]], m[s[3]]);
            mix(&mut v, [2, 6, 10, 14], m[s[4]], m[s[5]]);
            mix(&mut v, [3, 7, 11, 15], m[s[6]], m[s[7]]);
            mix(&mut v, [0, 5, 10, 15], m[s[8]], m[s[9]]);
            mix(&mut v, [1, 6, 11, 12], m[s[10]], m[s[11]]);
            mix(&mut v, [2, 7, 8, 13], m[s[12]], m[s[13]]);
            mix(&mut v, [3, 4, 9, 14], m[s[14]], m[s[15]]);
        }
        let mut state = [0u32; 8];
        for (i, word) in state.iter_mut().enumerate() {
            *word = v[i] ^ v[i + 8];
        }
        for (word, bytes) in state.iter_mut().zip(self.state.chunks_exact(4)) {
            *word ^= u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        self.store_state(&state);
        m.fill(0);
        v.fill(0);
        state.fill(0);
    }
}
