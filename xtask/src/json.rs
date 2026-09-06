//! Strict JSON parsing for caller-bounded host-tool output.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};

pub(crate) fn parse_json(bytes: &[u8], label: &str) -> Result<JsonValue> {
    let text = std::str::from_utf8(bytes).with_context(|| format!("{label} is not UTF-8"))?;
    let mut parser = JsonParser::new(text.as_bytes(), label);
    parser.whitespace();
    let value = parser.value(0)?;
    parser.whitespace();
    ensure!(parser.peek().is_none(), "{label} has trailing bytes");
    Ok(value)
}

pub(crate) fn json_object<'a>(
    value: &'a JsonValue,
    label: &str,
) -> Result<&'a BTreeMap<String, JsonValue>> {
    match value {
        JsonValue::Object(object) => Ok(object),
        _ => bail!("{label} is not an object"),
    }
}

pub(crate) fn json_array<'a>(value: &'a JsonValue, label: &str) -> Result<&'a [JsonValue]> {
    match value {
        JsonValue::Array(array) => Ok(array),
        _ => bail!("{label} is not an array"),
    }
}

pub(crate) fn json_string<'a>(value: &'a JsonValue, label: &str) -> Result<&'a str> {
    match value {
        JsonValue::String(string) => Ok(string),
        _ => bail!("{label} is not a string"),
    }
}

pub(crate) fn json_string_array<'a>(value: &'a JsonValue, label: &str) -> Result<Vec<&'a str>> {
    json_array(value, label)?
        .iter()
        .map(|value| json_string(value, label))
        .collect()
}

pub(crate) fn json_u64(value: &JsonValue, label: &str) -> Result<u64> {
    let JsonValue::Number(number) = value else {
        bail!("{label} is not a number");
    };
    let parsed = <u64 as core::str::FromStr>::from_str(number)
        .with_context(|| format!("{label} is not an unsigned integer"))?;
    ensure!(
        parsed.to_string() == *number,
        "{label} is not a canonical unsigned integer"
    );
    Ok(parsed)
}

pub(crate) fn json_field<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    name: &str,
    label: &str,
) -> Result<&'a JsonValue> {
    object
        .get(name)
        .with_context(|| format!("{label} has no `{name}` field"))
}

pub(crate) fn require_json_fields(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    label: &str,
) -> Result<()> {
    let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();
    ensure!(actual == expected, "{label} schema differs");
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    label: &'a str,
    offset: usize,
}

impl<'a> JsonParser<'a> {
    fn new(bytes: &'a [u8], label: &'a str) -> Self {
        Self {
            bytes,
            label,
            offset: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.offset += 1;
        Some(byte)
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.offset += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<JsonValue> {
        ensure!(depth <= 128, "{} nesting is excessive", self.label);
        self.whitespace();
        match self
            .peek()
            .with_context(|| format!("{} ended before a value", self.label))?
        {
            b'{' => self.object(depth + 1),
            b'[' => self.array(depth + 1),
            b'"' => self.string().map(JsonValue::String),
            b't' => {
                self.literal(b"true")?;
                Ok(JsonValue::Bool(true))
            }
            b'f' => {
                self.literal(b"false")?;
                Ok(JsonValue::Bool(false))
            }
            b'n' => {
                self.literal(b"null")?;
                Ok(JsonValue::Null)
            }
            b'-' | b'0'..=b'9' => self.number().map(JsonValue::Number),
            byte => bail!("{} has invalid value byte {byte}", self.label),
        }
    }

    fn object(&mut self, depth: usize) -> Result<JsonValue> {
        ensure!(
            self.take() == Some(b'{'),
            "{} object is malformed",
            self.label
        );
        let mut object = BTreeMap::new();
        self.whitespace();
        if self.peek() == Some(b'}') {
            self.take();
            return Ok(JsonValue::Object(object));
        }
        loop {
            let key = self.string()?;
            self.whitespace();
            ensure!(
                self.take() == Some(b':'),
                "{} object has no colon",
                self.label
            );
            let value = self.value(depth)?;
            ensure!(
                object.insert(key, value).is_none(),
                "{} contains a duplicate object key",
                self.label
            );
            self.whitespace();
            match self.take() {
                Some(b',') => self.whitespace(),
                Some(b'}') => return Ok(JsonValue::Object(object)),
                _ => bail!("{} object has no comma or close", self.label),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<JsonValue> {
        ensure!(
            self.take() == Some(b'['),
            "{} array is malformed",
            self.label
        );
        let mut array = Vec::new();
        self.whitespace();
        if self.peek() == Some(b']') {
            self.take();
            return Ok(JsonValue::Array(array));
        }
        loop {
            array.push(self.value(depth)?);
            self.whitespace();
            match self.take() {
                Some(b',') => self.whitespace(),
                Some(b']') => return Ok(JsonValue::Array(array)),
                _ => bail!("{} array has no comma or close", self.label),
            }
        }
    }

    fn string(&mut self) -> Result<String> {
        ensure!(
            self.take() == Some(b'"'),
            "{} string is malformed",
            self.label
        );
        let mut string = String::new();
        let mut segment = self.offset;
        loop {
            match self
                .take()
                .with_context(|| format!("{} string is unterminated", self.label))?
            {
                b'"' => {
                    string.push_str(
                        std::str::from_utf8(&self.bytes[segment..self.offset - 1])
                            .with_context(|| format!("{} string is not UTF-8", self.label))?,
                    );
                    return Ok(string);
                }
                b'\\' => {
                    string.push_str(
                        std::str::from_utf8(&self.bytes[segment..self.offset - 1])
                            .with_context(|| format!("{} string is not UTF-8", self.label))?,
                    );
                    match self
                        .take()
                        .with_context(|| format!("{} escape is unterminated", self.label))?
                    {
                        b'"' => string.push('"'),
                        b'\\' => string.push('\\'),
                        b'/' => string.push('/'),
                        b'b' => string.push('\u{0008}'),
                        b'f' => string.push('\u{000c}'),
                        b'n' => string.push('\n'),
                        b'r' => string.push('\r'),
                        b't' => string.push('\t'),
                        b'u' => string.push(self.unicode_escape()?),
                        _ => bail!("{} escape is invalid", self.label),
                    }
                    segment = self.offset;
                }
                0x00..=0x1f => bail!("{} string contains a control byte", self.label),
                _ => {}
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char> {
        let high = self.hex_quad()?;
        let scalar = if (0xd800..=0xdbff).contains(&high) {
            ensure!(
                self.take() == Some(b'\\') && self.take() == Some(b'u'),
                "{} Unicode surrogate is incomplete",
                self.label
            );
            let low = self.hex_quad()?;
            ensure!(
                (0xdc00..=0xdfff).contains(&low),
                "{} Unicode surrogate is malformed",
                self.label
            );
            0x1_0000 + (((high as u32 - 0xd800) << 10) | (low as u32 - 0xdc00))
        } else {
            ensure!(
                !(0xdc00..=0xdfff).contains(&high),
                "{} Unicode surrogate is malformed",
                self.label
            );
            high as u32
        };
        char::from_u32(scalar)
            .with_context(|| format!("{} Unicode escape is not a scalar", self.label))
    }

    fn hex_quad(&mut self) -> Result<u16> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = match self.take() {
                Some(b'0'..=b'9') => self.bytes[self.offset - 1] - b'0',
                Some(b'a'..=b'f') => self.bytes[self.offset - 1] - b'a' + 10,
                Some(b'A'..=b'F') => self.bytes[self.offset - 1] - b'A' + 10,
                _ => bail!("{} Unicode escape is malformed", self.label),
            };
            value = (value << 4) | u16::from(digit);
        }
        Ok(value)
    }

    fn literal(&mut self, expected: &[u8]) -> Result<()> {
        ensure!(
            self.bytes.get(self.offset..self.offset + expected.len()) == Some(expected),
            "{} literal is malformed",
            self.label
        );
        self.offset += expected.len();
        Ok(())
    }

    fn number(&mut self) -> Result<String> {
        let start = self.offset;
        if self.peek() == Some(b'-') {
            self.take();
        }
        match self.take() {
            Some(b'0') => ensure!(
                !self.peek().is_some_and(|byte| byte.is_ascii_digit()),
                "{} number has a leading zero",
                self.label
            ),
            Some(b'1'..=b'9') => {
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.take();
                }
            }
            _ => bail!("{} number has no integer", self.label),
        }
        if self.peek() == Some(b'.') {
            self.take();
            ensure!(
                self.peek().is_some_and(|byte| byte.is_ascii_digit()),
                "{} fraction has no digit",
                self.label
            );
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.take();
            }
        }
        if self.peek().is_some_and(|byte| matches!(byte, b'e' | b'E')) {
            self.take();
            if self.peek().is_some_and(|byte| matches!(byte, b'+' | b'-')) {
                self.take();
            }
            ensure!(
                self.peek().is_some_and(|byte| byte.is_ascii_digit()),
                "{} exponent has no digit",
                self.label
            );
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.take();
            }
        }
        Ok(std::str::from_utf8(&self.bytes[start..self.offset])
            .with_context(|| format!("{} number is not UTF-8", self.label))?
            .to_string())
    }
}
