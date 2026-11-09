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
