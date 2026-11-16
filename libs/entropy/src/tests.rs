use super::*;

fn hex<const N: usize>(text: &str) -> [u8; N] {
    let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
    assert_eq!(digits.len(), N * 2, "hex length");
    let mut out = [0u8; N];
    for (byte, pair) in out.iter_mut().zip(digits.chunks_exact(2)) {
        *byte = u8::from_str_radix(core::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    out
}

fn counting_key() -> [u8; 32] {
    core::array::from_fn(|i| i as u8)
}

#[test]
fn chacha20_block_matches_rfc_8439_section_2_3_2() {
    let nonce = hex("00 00 00 09 00 00 00 4a 00 00 00 00");
    let expected: [u8; 64] = hex(
        "10f1e7e4 d13b5915 500fdd1f a32071c4 c7d1f4c7 33c06803 0422aa9a c3d46c4e
         d2826446 079faa09 14c2d705 d98b02a2 b5129cd1 de164eb9 cbd083e8 a2503c4e",
    );
    assert_eq!(chacha::block(&counting_key(), 1, &nonce), expected, "block");
}

#[test]
fn chacha20_keystream_matches_rfc_8439_appendix_a_1_and_section_2_4_2() {
    let mut zero = [0u8; 128];
    chacha::keystream(&[0; 32], &mut zero);
    let expected: [u8; 128] = hex(
        "76b8e0ad a0f13d90 405d6ae5 5386bd28 bdd219b8 a08ded1a a836efcc 8b770dc7
         da41597c 5157488d 7724e03f b8d84a37 6a43b8f4 1518a11c c387b669 b2ee6586
         9f07e7be 5551387a 98ba977c 732d080d cb0f29a0 48e36569 12c6533e 32ee7aed
         29b72176 9ce64e43 d57133b0 74d839d5 31ed1f28 510afb45 ace10a1f 4b794d6f",
    );
    assert_eq!(zero, expected, "zero-key keystream");

    let nonce = hex("00 00 00 00 00 00 00 4a 00 00 00 00");
    let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let expected: [u8; 114] = hex(
        "6e2e359a 2568f980 41ba0728 dd0d6981 e97e7aec 1d4360c2 0a27afcc fd9fae0b
         f91b65c5 524733ab 8f593dab cd62b357 1639d624 e65152ab 8f530c35 9f0861d8
         07ca0dbf 500d6a61 56a38e08 8a22b65e 52bc514d 16ccf806 818ce91a b7793736
         5af90bbf 74a35be6 b40b8eed f2785e42 874d",
    );
    let mut cipher = [0u8; 114];
    for (i, piece) in plaintext.chunks(64).enumerate() {
        let block = chacha::block(&counting_key(), i as u32 + 1, &nonce);
        for (j, byte) in piece.iter().enumerate() {
            cipher[i * 64 + j] = byte ^ block[j];
        }
    }
    assert_eq!(cipher, expected, "sunscreen ciphertext");
}

#[test]
fn blake2s_matches_rfc_7693_appendix_b_and_a_keyed_vector() {
    let mut abc = Blake2s::new();
    abc.update(b"abc");
    let expected: [u8; 32] =
        hex("508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982");
    assert_eq!(abc.finalize(), expected, "abc");

    let keyed = Blake2s::keyed(&counting_key());
    let expected: [u8; 32] =
        hex("48a8997da407876b3d79c0d92325ad3b89cbb754d86ab71aee047ad345fd2c49");
    assert_eq!(keyed.finalize(), expected, "keyed empty");

    let mut keyed = Blake2s::keyed(&counting_key());
    keyed.update(&[0]);
    let expected: [u8; 32] =
        hex("40d15fee7c328830166ac3f918650f807e7e01e177258cdc0a39b11f598066f1");
    assert_eq!(keyed.finalize(), expected, "keyed one byte");
}

#[test]
fn no_output_before_256_credited_bits_and_every_output_replaces_the_key() {
    let mut base = Base::default();
    let mut local = Local::default();
    let mut out = [0u8; 80];

    assert!(!base.add_seed_word(0), "zero is rejected");
    assert!(!base.add_seed_word(u64::MAX), "all ones is rejected");
    for word in [0x1111_u64, 0x2222, 0x3333] {
        assert!(base.add_seed_word(word), "a fresh word is credited");
        base.mix_sample(word.rotate_left(7));
    }
    assert!(!base.add_seed_word(0x3333), "a repeat is not credited");
    assert!(!base.reseed(0), "192 credited bits are not enough");
    assert_eq!(local.fill(&mut base, &mut out), Err(NotReady), "192 bits");
    assert_eq!(out, [0u8; 80], "nothing written");

    assert!(base.add_seed_word(0x4444), "the fourth word is credited");
    assert_eq!(
        local.fill(&mut base, &mut out),
        Err(NotReady),
        "seeded only by a reseed"
    );
    assert!(base.reseed(0), "256 bits seed the key");
    assert_eq!(local.fill(&mut base, &mut out), Ok(()), "ready");
    assert_ne!(out, [0u8; 80], "output written");

    let key_before = local.key.0;
    let mut again = [0u8; 80];
    assert_eq!(local.fill(&mut base, &mut again), Ok(()), "second request");
    assert_ne!(local.key.0, key_before, "key replaced");
    assert_ne!(again, out, "fresh output");
}
