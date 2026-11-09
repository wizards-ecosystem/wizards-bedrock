use super::*;

#[test]
fn rendered_json_is_compact_sorted_and_round_trips() {
    let value = JsonValue::Object(BTreeMap::from([
        (
            "z".to_string(),
            JsonValue::Array(vec![
                JsonValue::Null,
                JsonValue::Bool(true),
                JsonValue::Number("-12.5e+2".to_string()),
                JsonValue::Bool(false),
            ]),
        ),
        ("a".to_string(), JsonValue::String("value".to_string())),
    ]));

    let rendered = render_json(&value).unwrap();
    assert_eq!(rendered, r#"{"a":"value","z":[null,true,-12.5e+2,false]}"#);
    assert_eq!(
        parse_json(rendered.as_bytes(), "rendered JSON").unwrap(),
        value
    );
}

#[test]
fn rendered_strings_escape_delimiters_controls_and_preserve_unicode() {
    let controls: String = (0..=0x1f).map(char::from).collect();
    let value = format!("C:\\scan\\\"quoted\"{controls}雪");
    let rendered = render_json(&JsonValue::String(value.clone())).unwrap();

    assert!(rendered.contains(r#"C:\\scan\\\"quoted\""#));
    for scalar in 0..=0x1f {
        assert!(
            rendered.contains(&format!("\\u{scalar:04x}")),
            "control U+{scalar:04X} was not escaped"
        );
    }
    assert_eq!(
        parse_json(rendered.as_bytes(), "rendered string").unwrap(),
        JsonValue::String(value)
    );
}

#[test]
fn invalid_number_lexemes_cannot_escape_the_renderer() {
    for number in ["", " 1", "01", "1 ", "1}", "NaN", "+1"] {
        assert!(
            render_json(&JsonValue::Number(number.to_string())).is_err(),
            "rendered invalid number `{number}`"
        );
    }
}

#[test]
fn manually_constructed_json_obeys_the_nesting_limit() {
    let mut value = JsonValue::Null;
    for _ in 0..130 {
        value = JsonValue::Array(vec![value]);
    }
    assert!(render_json(&value).is_err());
}

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
