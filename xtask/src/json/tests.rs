use super::*;

#[test]
fn parser_accepts_only_the_four_json_whitespace_bytes() {
    for bytes in [
        b" null".as_slice(),
        b"\tnull".as_slice(),
        b"\rnull".as_slice(),
        b"\nnull".as_slice(),
    ] {
        assert_eq!(parse_json(bytes, "whitespace").unwrap(), JsonValue::Null);
    }
    for bytes in [b"\x0bnull".as_slice(), b"\x0cnull".as_slice()] {
        assert!(parse_json(bytes, "non-JSON whitespace").is_err());
    }
}
