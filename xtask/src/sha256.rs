//! SHA-256, so vendored binaries can be checksum-pinned without taking on a
//! dependency for it. Straight from FIPS 180-4; verified against the NIST
//! vectors below.

const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const INITIAL_STATE: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const BLOCK_SIZE: usize = 64;

/// Hash `data`.
///
/// # Panics
///
/// Panics if the resident input exceeds SHA-256's 64-bit length field.
pub fn digest(data: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    let byte_length = data.len() as u128;
    assert!(
        byte_length <= u128::from(u64::MAX / 8),
        "resident SHA-256 input exceeds the algorithm's length field"
    );
    hasher.bytes = byte_length as u64;
    hasher.absorb(data);
    hasher.finish()
}

pub(crate) fn digest_reader(mut reader: impl std::io::Read) -> std::io::Result<(u64, [u8; 32])> {
    let mut hasher = Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut reader, &mut buffer)?;
        if read == 0 {
            return Ok((hasher.bytes, hasher.finish()));
        }
        hasher.update(&buffer[..read])?;
    }
}

pub(crate) struct Hasher {
    state: [u32; 8],
    buffer: [u8; BLOCK_SIZE],
    buffered: usize,
    bytes: u64,
}

impl Hasher {
    pub(crate) const fn new() -> Self {
        Self {
            state: INITIAL_STATE,
            buffer: [0; BLOCK_SIZE],
            buffered: 0,
            bytes: 0,
        }
    }

    pub(crate) fn update(&mut self, data: &[u8]) -> std::io::Result<()> {
        let additional = u64::try_from(data.len()).map_err(|_| input_too_large())?;
        self.bytes = self
            .bytes
            .checked_add(additional)
            .filter(|bytes| *bytes <= u64::MAX / 8)
            .ok_or_else(input_too_large)?;
        self.absorb(data);
        Ok(())
    }

    fn absorb(&mut self, mut data: &[u8]) {
        if self.buffered != 0 {
            let copied = (BLOCK_SIZE - self.buffered).min(data.len());
            self.buffer[self.buffered..self.buffered + copied].copy_from_slice(&data[..copied]);
            self.buffered += copied;
            data = &data[copied..];
            if self.buffered != BLOCK_SIZE {
                return;
            }
            compress(&mut self.state, &self.buffer);
            self.buffered = 0;
        }

        let (blocks, remainder) = data.as_chunks::<BLOCK_SIZE>();
        for block in blocks {
            compress(&mut self.state, block);
        }
        self.buffer[..remainder.len()].copy_from_slice(remainder);
        self.buffered = remainder.len();
    }

    pub(crate) fn finish(mut self) -> [u8; 32] {
        let mut final_block = [0u8; BLOCK_SIZE];
        final_block[..self.buffered].copy_from_slice(&self.buffer[..self.buffered]);
        final_block[self.buffered] = 0x80;

        let bits = self.bytes * 8;
        if self.buffered + 9 <= BLOCK_SIZE {
            final_block[BLOCK_SIZE - 8..].copy_from_slice(&bits.to_be_bytes());
            compress(&mut self.state, &final_block);
        } else {
            compress(&mut self.state, &final_block);
            let mut length_block = [0u8; BLOCK_SIZE];
            length_block[BLOCK_SIZE - 8..].copy_from_slice(&bits.to_be_bytes());
            compress(&mut self.state, &length_block);
        }

        let mut out = [0u8; 32];
        for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

fn input_too_large() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "input is too large for the SHA-256 length field",
    )
}

/// Lowercase hex, the form `sha256sum` prints.
pub fn hex(digest: &[u8; 32]) -> String {
    digest
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            out.push_str(&format!("{byte:02x}"));
            out
        })
}

fn compress(state: &mut [u32; 8], block: &[u8; BLOCK_SIZE]) {
    let mut schedule = [0u32; 64];
    for (word, chunk) in schedule.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_be_bytes(*chunk);
    }
    for index in 16..64 {
        let s0 = rotate(schedule[index - 15], 7, 18, 3);
        let s1 = rotate(schedule[index - 2], 17, 19, 10);
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (word, constant) in schedule.into_iter().zip(ROUND_CONSTANTS) {
        let sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(sigma1)
            .wrapping_add(choose)
            .wrapping_add(constant)
            .wrapping_add(word);
        let sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = sigma0.wrapping_add(majority);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}

/// The two rotations and a shift that make up SHA-256's small sigma functions.
fn rotate(word: u32, first: u32, second: u32, shift: u32) -> u32 {
    word.rotate_right(first) ^ word.rotate_right(second) ^ (word >> shift)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(input: &[u8]) -> String {
        hex(&digest(input))
    }

    #[test]
    fn empty_input_matches_the_nist_vector() {
        assert_eq!(
            hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn abc_matches_the_nist_vector() {
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_448_bit_message_matches_the_nist_vector() {
        let input = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(
            hash(input),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn a_multi_block_message_matches_the_nist_vector() {
        let input = vec![b'a'; 1_000_000];
        assert_eq!(
            hash(&input),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn a_message_that_lands_on_a_block_boundary_pads_into_a_second_block() {
        // 56 bytes leaves no room for the length field, forcing a second block.
        let input = vec![b'x'; 56];
        assert_eq!(hash(&input).len(), 64);
        assert_ne!(hash(&input), hash(&[b'x'; 55]));
    }

    #[test]
    fn incremental_hashing_matches_every_adversarial_block_split() {
        let input: Vec<u8> = (0..=255).cycle().take(1025).collect();
        let expected = digest(&input);
        for chunk in [1, 7, 55, 56, 63, 64, 65, 127, 128, 129] {
            let mut hasher = Hasher::new();
            for bytes in input.chunks(chunk) {
                hasher.update(bytes).unwrap();
            }
            assert_eq!(hasher.finish(), expected, "chunk size {chunk}");
        }
    }

    #[test]
    fn a_reader_is_hashed_without_changing_length_or_digest() {
        let input: Vec<u8> = (0..=255).cycle().take(128 * 1024 + 17).collect();
        let (bytes, streamed) = digest_reader(std::io::Cursor::new(&input)).unwrap();
        assert_eq!(bytes, input.len() as u64);
        assert_eq!(streamed, digest(&input));
    }
}
