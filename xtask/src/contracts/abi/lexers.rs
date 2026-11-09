fn cpp_raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start..start + 2) != Some(b"R\"") {
        return None;
    }
    let identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let boundary = |at: usize| at == 0 || !identifier(bytes[at - 1]);
    let has_prefix = boundary(start)
        || (start >= 1 && matches!(bytes[start - 1], b'u' | b'U' | b'L') && boundary(start - 1))
        || (start >= 2 && &bytes[start - 2..start] == b"u8" && boundary(start - 2));
    if !has_prefix {
        return None;
    }
    let delimiter_at = start + 2;
    let open = bytes[delimiter_at..]
        .iter()
        .position(|byte| *byte == b'(')
        .map(|offset| delimiter_at + offset)
        .expect("unterminated C++ raw-string delimiter");
    let delimiter = &bytes[delimiter_at..open];
    assert!(delimiter.len() <= 16, "oversized C++ raw-string delimiter");
    assert!(
        delimiter
            .iter()
            .all(|byte| { !byte.is_ascii_whitespace() && !matches!(*byte, b'(' | b')' | b'\\') }),
        "invalid C++ raw-string delimiter"
    );
    let terminator_bytes = 2 + delimiter.len();
    let close = bytes[open + 1..]
        .windows(terminator_bytes)
        .position(|window| {
            window[0] == b')'
                && &window[1..1 + delimiter.len()] == delimiter
                && window[1 + delimiter.len()] == b'"'
        })
        .map(|offset| open + 1 + offset + terminator_bytes)
        .expect("unterminated C++ raw string literal");
    Some(close)
}

pub(super) fn cpp_without_comments(source: &str) -> (String, Vec<String>) {
    for trigraph in [
        "??=", "??/", "??'", "??(", "??)", "??!", "??<", "??>", "??-",
    ] {
        assert!(
            !source.contains(trigraph),
            "C/C++ trigraph {trigraph} is not admitted"
        );
    }
    for (line_number, line) in source.lines().enumerate() {
        assert!(
            !line.trim_end().ends_with('\\'),
            "C/C++ line continuation on line {}",
            line_number + 1
        );
    }
    let bytes = source.as_bytes();
    let mut active = bytes.to_vec();
    let mut line_comments = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if let Some(end) = cpp_raw_string_end(bytes, index) {
            index = end;
            continue;
        }
        match bytes[index] {
            quote @ (b'\'' | b'"') => {
                index += 1;
                let mut closed = false;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == quote {
                        index += 1;
                        closed = true;
                        break;
                    } else {
                        index += 1;
                    }
                }
                assert!(closed, "unterminated C++ literal");
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let end = bytes[index + 2..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(bytes.len(), |offset| index + 2 + offset);
                line_comments.push(
                    String::from_utf8_lossy(&bytes[index + 2..end])
                        .trim()
                        .to_string(),
                );
                active[index..end].fill(b' ');
                index = end;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                index += 2;
                let mut closed = false;
                while index + 1 < bytes.len() {
                    if bytes[index] == b'*' && bytes[index + 1] == b'/' {
                        index += 2;
                        closed = true;
                        break;
                    }
                    index += 1;
                }
                assert!(closed, "unterminated C++ block comment");
                for byte in &mut active[start..index] {
                    if *byte != b'\n' && *byte != b'\r' {
                        *byte = b' ';
                    }
                }
            }
            _ => index += 1,
        }
    }

    (
        String::from_utf8(active).expect("comment masking kept UTF-8"),
        line_comments,
    )
}

pub(super) fn cpp_structure(active: &str) -> String {
    let bytes = active.as_bytes();
    let mut structure = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = cpp_raw_string_end(bytes, index) {
            for byte in &mut structure[index..end] {
                if *byte != b'\n' && *byte != b'\r' {
                    *byte = b' ';
                }
            }
            index = end;
            continue;
        }
        let quote = bytes[index];
        if quote != b'\'' && quote != b'"' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        let mut closed = false;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
            } else if bytes[index] == quote {
                index += 1;
                closed = true;
                break;
            } else {
                index += 1;
            }
        }
        assert!(closed, "unterminated C++ literal");
        for byte in &mut structure[start..index] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(structure).expect("literal masking kept UTF-8")
}

pub(crate) fn rust_without_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut active = bytes.to_vec();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'r' {
            let mut quote = index + 1;
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
            if bytes.get(quote) == Some(&b'"') {
                let hashes = quote - index - 1;
                let mut end = quote + 1;
                let mut closed = None;
                while end < bytes.len() {
                    if bytes[end] == b'"'
                        && bytes.get(end + 1..end + 1 + hashes) == Some(&bytes[index + 1..quote])
                    {
                        closed = Some(end + 1 + hashes);
                        break;
                    }
                    end += 1;
                }
                let end = closed.expect("unterminated Rust raw string literal");
                for byte in &mut active[index..end] {
                    if *byte != b'\n' && *byte != b'\r' {
                        *byte = b' ';
                    }
                }
                index = end;
                continue;
            }
        }
        match bytes[index] {
            b'"' => {
                let start = index;
                index += 1;
                let mut closed = false;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == b'"' {
                        index += 1;
                        closed = true;
                        break;
                    } else {
                        index += 1;
                    }
                }
                assert!(closed, "unterminated Rust string literal");
                for byte in &mut active[start..index] {
                    if *byte != b'\n' && *byte != b'\r' {
                        *byte = b' ';
                    }
                }
            }
            b'\'' => {
                let start = index;
                let mut cursor = index + 1;
                let mut escaped = false;
                let mut close = None;
                while cursor < bytes.len() && bytes[cursor] != b'\n' {
                    if !escaped && bytes[cursor] == b'\'' {
                        close = Some(cursor + 1);
                        break;
                    }
                    escaped = !escaped && bytes[cursor] == b'\\';
                    cursor += 1;
                }
                let is_character = close.is_some_and(|end| {
                    let contents = &source[start + 1..end - 1];
                    contents.starts_with('\\') || contents.chars().count() == 1
                });
                if is_character {
                    index = close.expect("a character literal has a close");
                    active[start..index].fill(b' ');
                } else {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let end = bytes[index + 2..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(bytes.len(), |offset| index + 2 + offset);
                active[index..end].fill(b' ');
                index = end;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                let mut depth = 1usize;
                index += 2;
                while index < bytes.len() && depth != 0 {
                    if bytes.get(index + 1) == Some(&b'*') && bytes[index] == b'/' {
                        depth += 1;
                        index += 2;
                    } else if bytes.get(index + 1) == Some(&b'/') && bytes[index] == b'*' {
                        depth -= 1;
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                assert_eq!(depth, 0, "unterminated Rust block comment");
                for byte in &mut active[start..index] {
                    if *byte != b'\n' && *byte != b'\r' {
                        *byte = b' ';
                    }
                }
            }
            _ => index += 1,
        }
    }

    String::from_utf8(active).expect("Rust comment masking kept UTF-8")
}

pub(super) fn cpp_directive(line: &str) -> Option<(&str, &str)> {
    let directive = line.trim_start().strip_prefix('#')?.trim_start();
    let split = directive
        .bytes()
        .position(|byte| byte.is_ascii_whitespace())
        .unwrap_or(directive.len());
    Some((&directive[..split], directive[split..].trim()))
}

pub(super) fn checked_header(source: &str) -> (String, std::collections::BTreeMap<String, String>) {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Phase {
        BeforeGuard,
        GuardDefinition,
        Includes,
        LinkageOpen,
        Body,
        LinkageClose,
        GuardClose,
        Done,
    }

    let (active, _) = cpp_without_comments(source);
    let structure = cpp_structure(&active);
    let mut macros = std::collections::BTreeMap::new();
    let mut includes = Vec::new();
    let mut linkage_body = String::new();
    let mut phase = Phase::BeforeGuard;

    for (line_number, (line, structural_line)) in active.lines().zip(structure.lines()).enumerate()
    {
        let structural_line = structural_line.trim();
        assert!(
            !structural_line.starts_with("%:") && !structural_line.starts_with("??="),
            "alternate preprocessor directive on line {}",
            line_number + 1
        );
        assert!(
            !structural_line
                .split(|character: char| { !character.is_ascii_alphanumeric() && character != '_' })
                .any(|word| matches!(word, "_Pragma" | "__pragma")),
            "pragma operator on line {}",
            line_number + 1
        );
        if cpp_directive(structural_line).is_some() {
            let (kind, rest) = cpp_directive(line).expect("structural directive has source");
            assert!(
                !line.trim_end().ends_with('\\'),
                "continued header directive on line {}",
                line_number + 1
            );
            match kind {
                "ifndef" => {
                    assert_eq!(phase, Phase::BeforeGuard, "misplaced include guard");
                    assert_eq!(rest, "WIZARD_ABI_H", "unexpected header guard");
                    phase = Phase::GuardDefinition;
                }
                "ifdef" => {
                    assert_eq!(rest, "__cplusplus", "unexpected header conditional");
                    phase = match phase {
                        Phase::Includes => {
                            assert_eq!(
                                includes,
                                ["<stddef.h>", "<stdint.h>"],
                                "the fixed header includes changed"
                            );
                            linkage_body.clear();
                            Phase::LinkageOpen
                        }
                        Phase::Body => {
                            linkage_body.clear();
                            Phase::LinkageClose
                        }
                        _ => panic!("misplaced __cplusplus guard"),
                    };
                }
                "endif" => {
                    assert!(rest.is_empty(), "unexpected tokens after #endif");
                    phase = match phase {
                        Phase::LinkageOpen => {
                            assert_eq!(
                                linkage_body, "extern\"C\"{",
                                "the first __cplusplus guard is not the linkage opener"
                            );
                            Phase::Body
                        }
                        Phase::LinkageClose => {
                            assert_eq!(
                                linkage_body, "}",
                                "the second __cplusplus guard is not the linkage closer"
                            );
                            Phase::GuardClose
                        }
                        Phase::GuardClose => Phase::Done,
                        _ => panic!("#endif without the expected opener"),
                    };
                }
                "include" => {
                    assert_eq!(phase, Phase::Includes, "misplaced header include");
                    assert!(
                        matches!(rest, "<stddef.h>" | "<stdint.h>"),
                        "unexpected header include `{rest}`"
                    );
                    assert!(!includes.contains(&rest), "duplicate include `{rest}`");
                    includes.push(rest);
                }
                "define" => {
                    let split = rest
                        .bytes()
                        .position(|byte| byte.is_ascii_whitespace())
                        .unwrap_or(rest.len());
                    let name = &rest[..split];
                    let value = rest[split..].trim();
                    if name == "WIZARD_ABI_H" {
                        assert_eq!(
                            phase,
                            Phase::GuardDefinition,
                            "misplaced include guard definition"
                        );
                        assert!(value.is_empty(), "the include guard has a value");
                        phase = Phase::Includes;
                        continue;
                    }
                    assert_eq!(phase, Phase::Body, "macro outside the linkage body");
                    assert!(name.starts_with("WIZ_"), "unexpected macro `{name}`");
                    assert!(
                        name.bytes().all(|byte| byte.is_ascii_uppercase()
                            || byte.is_ascii_digit()
                            || byte == b'_'),
                        "non-object macro `{name}`"
                    );
                    assert!(!value.is_empty(), "macro `{name}` has no value");
                    assert!(
                        macros.insert(name.to_string(), value.to_string()).is_none(),
                        "duplicate macro `{name}`"
                    );
                }
                other => panic!("unexpected header directive #{other}"),
            }
            continue;
        }

        let code = compact_cpp(line);
        match phase {
            Phase::LinkageOpen | Phase::LinkageClose => linkage_body.push_str(&code),
            Phase::Body => {}
            _ => assert!(
                code.is_empty(),
                "code outside the header's declared linkage body"
            ),
        }
    }

    assert_eq!(phase, Phase::Done, "incomplete header conditional scaffold");
    (active, macros)
}

pub(super) fn matching(source: &[u8], open: usize, left: u8, right: u8) -> usize {
    let mut depth = 0usize;
    for (offset, byte) in source[open..].iter().enumerate() {
        if *byte == left {
            depth += 1;
        } else if *byte == right {
            depth = depth.checked_sub(1).expect("closing delimiter before open");
            if depth == 0 {
                return open + offset;
            }
        }
    }
    panic!("unterminated delimiter at byte {open}");
}

pub(super) fn cpp_bodies(active: &str) -> std::collections::BTreeMap<String, (&str, &str)> {
    let structure = cpp_structure(active);
    let bytes = structure.as_bytes();
    let outer_marker = "namespace mlibc";
    let outer_markers: Vec<_> = structure.match_indices(outer_marker).collect();
    assert_eq!(outer_markers.len(), 1, "expected one namespace mlibc");
    assert!(
        outer_markers[0].0 == 0
            || !bytes[outer_markers[0].0 - 1].is_ascii_alphanumeric()
                && bytes[outer_markers[0].0 - 1] != b'_',
        "namespace mlibc is embedded in an identifier"
    );
    let mut outer_open = outer_markers[0].0 + outer_marker.len();
    while bytes.get(outer_open).is_some_and(u8::is_ascii_whitespace) {
        outer_open += 1;
    }
    assert_eq!(
        bytes.get(outer_open),
        Some(&b'{'),
        "namespace mlibc has unexpected qualifiers"
    );
    let outer_close = matching(bytes, outer_open, b'{', b'}');
    assert!(
        structure[outer_close + 1..].trim().is_empty(),
        "active C++ follows namespace mlibc"
    );
    let inner_marker = "namespace {";
    let inner_markers: Vec<_> = structure[outer_open + 1..outer_close]
        .match_indices(inner_marker)
        .map(|(at, marker)| (outer_open + 1 + at, marker))
        .filter(|(at, _)| {
            *at == 0 || !bytes[*at - 1].is_ascii_alphanumeric() && bytes[*at - 1] != b'_'
        })
        .collect();
    assert_eq!(inner_markers.len(), 1, "expected one anonymous namespace");
    let inner_open = inner_markers[0].0 + inner_marker.len() - 1;
    let inner_close = matching(bytes, inner_open, b'{', b'}');
    assert!(
        inner_close < outer_close,
        "anonymous namespace escapes mlibc"
    );
    let mut bodies = std::collections::BTreeMap::new();
    let mut index = 0usize;
    let mut depth = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'{' {
            depth += 1;
            index += 1;
            continue;
        }
        if bytes[index] == b'}' {
            depth = depth.checked_sub(1).expect("unmatched C++ closing brace");
            index += 1;
            continue;
        }
        if !bytes[index].is_ascii_alphabetic() && bytes[index] != b'_' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        let name = &structure[start..index];
        if !name.starts_with("sys_")
            && !matches!(
                name,
                "call1" | "as_errno" | "read_counter" | "monotonic_fallback"
            )
        {
            continue;
        }
        let mut cursor = index;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'(') {
            continue;
        }
        let close_parameters = matching(bytes, cursor, b'(', b')');
        cursor = close_parameters + 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'{') {
            continue;
        }
        let (expected_depth, in_expected_namespace) = if name.starts_with("sys_") {
            (
                1,
                start > outer_open
                    && start < outer_close
                    && !(start > inner_open && start < inner_close),
            )
        } else {
            (2, start > inner_open && start < inner_close)
        };
        assert_eq!(
            depth, expected_depth,
            "{name} is not defined at its expected mlibc namespace depth"
        );
        assert!(
            in_expected_namespace,
            "{name} is outside its expected mlibc namespace"
        );
        let close_body = matching(bytes, cursor, b'{', b'}');
        let declaration_start = structure[..start]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        assert!(
            bodies
                .insert(
                    name.to_string(),
                    (
                        active[declaration_start..cursor].trim(),
                        &active[cursor + 1..close_body],
                    ),
                )
                .is_none(),
            "two active definitions of {name}"
        );
        index = close_body + 1;
    }
    assert_eq!(depth, 0, "unbalanced C++ braces");
    bodies
}

pub(super) fn sysdep_annotations(
    line_comments: &[String],
) -> std::collections::BTreeMap<String, (String, String)> {
    let mut annotations = std::collections::BTreeMap::new();
    for comment in line_comments {
        let Some(rest) = comment.strip_prefix("sys_") else {
            continue;
        };
        let Some((name, states)) = rest.split_once(". BODY=") else {
            continue;
        };
        let (body, remainder) = states
            .split_once("; REMAINDER=")
            .unwrap_or_else(|| panic!("sys_{name} has no REMAINDER state"));
        let remainder = remainder
            .strip_suffix('.')
            .unwrap_or_else(|| panic!("sys_{name}'s state marker has no full stop"));
        assert!(
            matches!(body, "WIRED" | "COMPOSED" | "PARTIAL" | "STUB"),
            "sys_{name} has unknown BODY={body}"
        );
        assert!(
            matches!(
                remainder,
                "WORKS" | "COMPAT" | "DOOR" | "PARTIAL" | "MISSING"
            ),
            "sys_{name} has unknown REMAINDER={remainder}"
        );
        assert!(
            annotations
                .insert(
                    format!("sys_{name}"),
                    (body.to_string(), remainder.to_string())
                )
                .is_none(),
            "sys_{name} has two state markers"
        );
    }
    annotations
}

pub(crate) fn compact_cpp(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

pub(super) fn assert_clock_arithmetic(source: &str) {
    let compact = compact_cpp(&cpp_structure(source));
    let start = compact
        .find("uint64_tticks=")
        .expect("clock counter-delta calculation");
    let end = compact[start..]
        .find("*secs=")
        .map(|offset| start + offset)
        .expect("clock result stores");
    assert_eq!(
        &compact[start..end],
        concat!(
            "uint64_tticks=stamp-last;",
            "unsigned__int128advanced=static_cast<unsigned__int128>(fraction)",
            "+static_cast<unsigned__int128>(ticks)*multiplier;",
            "unsigned__int128whole=advanced>>shift;",
            "uint64_tns;",
            "if(whole>UINT64_MAX){ns=UINT64_MAX;}",
            "else{uint64_tdelta=static_cast<uint64_t>(whole);",
            "ns=base>UINT64_MAX-delta?UINT64_MAX:base+delta;}"
        ),
        "the clock arithmetic no longer widens before multiplying and saturates afterwards"
    );
}

struct CIntegerParser<'a> {
    source: &'a [u8],
    at: usize,
}

impl CIntegerParser<'_> {
    fn skip_space(&mut self) {
        while self
            .source
            .get(self.at)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.at += 1;
        }
    }

    fn take(&mut self, token: &[u8]) -> bool {
        self.skip_space();
        if self.source.get(self.at..self.at + token.len()) == Some(token) {
            self.at += token.len();
            true
        } else {
            false
        }
    }

    fn identifier(&mut self) -> Option<String> {
        self.skip_space();
        let start = self.at;
        if !self
            .source
            .get(self.at)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        {
            return None;
        }
        self.at += 1;
        while self
            .source
            .get(self.at)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            self.at += 1;
        }
        Some(
            std::str::from_utf8(&self.source[start..self.at])
                .expect("ASCII identifier")
                .to_string(),
        )
    }

    fn number(&mut self) -> Result<u64, String> {
        self.skip_space();
        let start = self.at;
        let (radix, digits_at) = if self.source.get(self.at..self.at + 2) == Some(b"0x")
            || self.source.get(self.at..self.at + 2) == Some(b"0X")
        {
            self.at += 2;
            (16, self.at)
        } else {
            (10, self.at)
        };
        while self.source.get(self.at).is_some_and(|byte| match radix {
            16 => byte.is_ascii_hexdigit(),
            _ => byte.is_ascii_digit(),
        }) {
            self.at += 1;
        }
        if self.at == digits_at {
            return Err(format!("expected an integer literal at byte {start}"));
        }
        let digits =
            std::str::from_utf8(&self.source[digits_at..self.at]).expect("ASCII integer literal");
        if radix == 10 && digits.len() > 1 && digits.starts_with('0') {
            return Err(format!(
                "octal-looking integer literal `{digits}` is not admitted"
            ));
        }
        let suffix_at = self.at;
        while self
            .source
            .get(self.at)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            self.at += 1;
        }
        let suffix = std::str::from_utf8(&self.source[suffix_at..self.at])
            .expect("ASCII integer suffix")
            .to_ascii_lowercase();
        if !matches!(
            suffix.as_str(),
            "" | "u" | "l" | "ul" | "lu" | "ll" | "ull" | "llu"
        ) {
            return Err(format!("unsupported integer suffix `{suffix}`"));
        }
        u64::from_str_radix(digits, radix)
            .map_err(|_| format!("integer literal `{digits}` does not fit in u64"))
    }

    fn primary(&mut self) -> Result<u64, String> {
        self.skip_space();
        if !self.take(b"(") {
            return self.number();
        }

        let inside = self.at;
        if let Some(kind) = self.identifier() {
            self.skip_space();
            if self.take(b")") {
                if !matches!(
                    kind.as_str(),
                    "wiz_handle_t" | "wiz_rights_t" | "uintptr_t" | "uint64_t"
                ) {
                    return Err(format!("unsupported C cast `{kind}`"));
                }
                return self.primary();
            }
        }

        self.at = inside;
        let value = self.shift()?;
        if !self.take(b")") {
            return Err(format!(
                "unterminated parenthesized integer at byte {inside}"
            ));
        }
        Ok(value)
    }

    fn shift(&mut self) -> Result<u64, String> {
        let mut value = self.primary()?;
        while self.take(b"<<") {
            let shift = self.primary()?;
            if shift >= u64::from(u32::BITS) || value > (u64::from(u32::MAX) >> shift) {
                return Err("overflowing integer shift".to_string());
            }
            value <<= shift;
        }
        Ok(value)
    }
}

pub(super) fn parse_c_uint(expression: &str) -> Result<u64, String> {
    let mut parser = CIntegerParser {
        source: expression.as_bytes(),
        at: 0,
    };
    let value = parser.shift()?;
    parser.skip_space();
    if parser.at != parser.source.len() {
        return Err(format!(
            "unparsed C integer expression `{}`",
            &expression[parser.at..]
        ));
    }
    Ok(value)
}

pub(super) fn parse_rust_uint(literal: &str) -> Result<u64, String> {
    let literal = literal.trim();
    let (radix, prefix) = if literal.starts_with("0x") {
        (16, 2)
    } else if literal.starts_with("0b") {
        (2, 2)
    } else if literal.starts_with("0o") {
        (8, 2)
    } else {
        (10, 0)
    };
    let mut end = prefix;
    for (offset, character) in literal[prefix..].char_indices() {
        let digit = character == '_'
            || match radix {
                16 => character.is_ascii_hexdigit(),
                10 => character.is_ascii_digit(),
                8 => matches!(character, '0'..='7'),
                2 => matches!(character, '0' | '1'),
                _ => unreachable!(),
            };
        if !digit {
            break;
        }
        end = prefix + offset + character.len_utf8();
    }
    if end == prefix {
        return Err(format!("`{literal}` is not an unsigned Rust literal"));
    }
    let suffix = &literal[end..];
    if !matches!(
        suffix,
        "" | "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize"
    ) {
        return Err(format!("unsupported Rust integer suffix `{suffix}`"));
    }
    let digits: String = literal[prefix..end]
        .chars()
        .filter(|character| *character != '_')
        .collect();
    u64::from_str_radix(&digits, radix)
        .map_err(|_| format!("Rust integer literal `{literal}` does not fit in u64"))
}

pub(super) fn rust_delimiter_depth(source: &str, end: usize) -> [usize; 3] {
    let mut depth = [0usize; 3];
    for byte in &source.as_bytes()[..end] {
        match byte {
            b'{' => depth[0] += 1,
            b'}' => {
                depth[0] = depth[0]
                    .checked_sub(1)
                    .expect("unmatched Rust closing brace")
            }
            b'(' => depth[1] += 1,
            b')' => {
                depth[1] = depth[1]
                    .checked_sub(1)
                    .expect("unmatched Rust closing parenthesis")
            }
            b'[' => depth[2] += 1,
            b']' => {
                depth[2] = depth[2]
                    .checked_sub(1)
                    .expect("unmatched Rust closing bracket")
            }
            _ => {}
        }
    }
    depth
}

fn immediately_preceding_outer_rust_attribute(source: &str, line_start: usize) -> bool {
    let prefix = source[..line_start].trim_end();
    let bytes = prefix.as_bytes();
    if bytes.last() != Some(&b']') {
        return false;
    }
    let mut depth = 0usize;
    for at in (0..bytes.len()).rev() {
        match bytes[at] {
            b']' => depth += 1,
            b'[' => {
                depth = depth
                    .checked_sub(1)
                    .expect("opening Rust bracket before its close");
                if depth == 0 {
                    return bytes.get(at.wrapping_sub(1)) == Some(&b'#');
                }
            }
            _ => {}
        }
    }
    panic!("unterminated Rust attribute before an ABI witness");
}

pub(super) fn assert_unconditional_rust_line(source: &str, at: usize, depth: usize, label: &str) {
    let line_start = source[..at].rfind('\n').map_or(0, |newline| newline + 1);
    assert!(
        source[line_start..at].trim().is_empty(),
        "{label} is not a direct Rust item"
    );
    let nesting = rust_delimiter_depth(source, at);
    assert_eq!(nesting[0], depth, "{label} is at the wrong brace depth");
    assert_eq!(
        [nesting[1], nesting[2]],
        [0, 0],
        "{label} is nested in a Rust macro or expression"
    );
    assert!(
        !immediately_preceding_outer_rust_attribute(source, line_start),
        "{label} is controlled by a Rust attribute"
    );
}

pub(super) fn rust_expression(source: &str, declaration: &str) -> String {
    let source = rust_without_comments(source);
    let prefix = format!("{declaration} = ");
    let matches: Vec<_> = source.match_indices(&prefix).collect();
    assert_eq!(matches.len(), 1, "expected one `{declaration}` declaration");
    let at = matches[0].0;
    assert_unconditional_rust_line(&source, at, 0, declaration);
    let value_start = at + prefix.len();
    let line_end = source[value_start..]
        .find('\n')
        .map_or(source.len(), |offset| value_start + offset);
    source[value_start..line_end]
        .trim()
        .strip_suffix(';')
        .unwrap_or_else(|| panic!("unterminated `{declaration}` declaration"))
        .trim()
        .to_string()
}

pub(crate) fn rust_function_body(source: &str, signature: &str) -> String {
    let source = rust_without_comments(source);
    let matches: Vec<_> = source.match_indices(signature).collect();
    assert_eq!(matches.len(), 1, "expected one `{signature}`");
    let at = matches[0].0;
    assert_unconditional_rust_line(&source, at, 0, signature);
    let open = at + signature.len() - 1;
    assert_eq!(source.as_bytes().get(open), Some(&b'{'));
    let close = matching(source.as_bytes(), open, b'{', b'}');
    source[open + 1..close].to_string()
}

pub(super) fn rust_uint(source: &str, declaration: &str) -> u64 {
    let expression = rust_expression(source, declaration);
    parse_rust_uint(&expression).unwrap_or_else(|error| panic!("unparsed `{declaration}`: {error}"))
}
