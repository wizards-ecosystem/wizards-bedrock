use super::*;

pub(super) fn contains_diagnostic_marker(text: &str) -> Result<bool> {
    if bytes_contain_diagnostic_marker(text.as_bytes()) {
        return Ok(true);
    }
    let tokens = <TokenStream as core::str::FromStr>::from_str(text).map_err(|error| {
        anyhow::anyhow!(
            "tokenizing Rust source while scanning for the reserved API marker: {error}"
        )
    })?;
    token_stream_contains_diagnostic_marker(tokens)
}

fn token_stream_contains_diagnostic_marker(tokens: TokenStream) -> Result<bool> {
    for token in tokens {
        match token {
            TokenTree::Group(group) => {
                if token_stream_contains_diagnostic_marker(group.stream())? {
                    return Ok(true);
                }
            }
            TokenTree::Literal(literal) => {
                let token = TokenStream::from(TokenTree::Literal(literal));
                let literal = syn::parse2::<Lit>(token).context(
                    "decoding a Rust literal while scanning for the reserved API marker",
                )?;
                let contains = match literal {
                    Lit::Str(value) => bytes_contain_diagnostic_marker(value.value().as_bytes()),
                    Lit::ByteStr(value) => bytes_contain_diagnostic_marker(&value.value()),
                    Lit::CStr(value) => bytes_contain_diagnostic_marker(value.value().as_bytes()),
                    _ => false,
                };
                if contains {
                    return Ok(true);
                }
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
    }
    Ok(false)
}

pub(super) fn bytes_contain_diagnostic_marker(bytes: &[u8]) -> bool {
    bytes.windows(MARKER.len() + 1).any(|window| {
        window.starts_with(MARKER.as_bytes()) && window[MARKER.len()].is_ascii_digit()
    })
}

fn authority_file_contains_diagnostic_marker(path: &Path, bytes: &[u8]) -> Result<bool> {
    if bytes_contain_diagnostic_marker(bytes) {
        return Ok(true);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        ensure!(
            path.extension().is_none_or(|extension| extension != "rs"),
            "Rust compiler input {} is not UTF-8",
            path.display()
        );
        return Ok(false);
    };
    if path.extension().is_some_and(|extension| extension == "rs") {
        return contains_diagnostic_marker(text);
    }
    let Ok(tokens) = <TokenStream as core::str::FromStr>::from_str(text) else {
        return Ok(false);
    };
    token_stream_contains_diagnostic_marker(tokens)
}

pub(super) fn reject_authority_diagnostic_marker(
    path: &Path,
    bytes: &[u8],
    label: &str,
) -> Result<()> {
    ensure!(
        !authority_file_contains_diagnostic_marker(path, bytes)
            .with_context(|| format!("scanning {} for the reserved API marker", path.display()))?,
        "{label} {} contains the API audit's reserved diagnostic marker",
        path.display()
    );
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MarkerObservation {
    pub(super) file: usize,
    pub(super) id: usize,
    pub(super) source_line: usize,
    pub(super) column: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct CargoDiagnosticOrigin {
    pub(super) package_id: String,
    pub(super) target_name: String,
    pub(super) target_kind: CargoTargetKind,
    pub(super) target_source: PathBuf,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct MarkerDiagnostic {
    pub(super) id: usize,
    pub(super) origin: CargoDiagnosticOrigin,
    pub(super) reported: String,
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) immediate_expansion: Option<DiagnosticExpansion>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct DiagnosticLocation {
    pub(super) reported: String,
    pub(super) line: usize,
    pub(super) column: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct DiagnosticExpansion {
    pub(super) location: DiagnosticLocation,
    pub(super) terminal: bool,
    pub(super) macro_decl_name: String,
}

pub(super) fn validate_marker_round(
    id: usize,
    inventory: &Inventory,
    instrumentation: &Instrumentation,
) -> Result<()> {
    ensure!(
        id < inventory.nodes.len(),
        "an API diagnostic names unknown item {id}"
    );
    ensure!(
        instrumentation.ids.contains(&id),
        "an API diagnostic names item {id}, which was not instrumented in this compiler round"
    );
    Ok(())
}

pub(super) fn corrected_marker_location(
    file: usize,
    source_line: usize,
    column: usize,
    instrumentation: &Instrumentation,
) -> Result<LineColumn> {
    ensure!(
        source_line > 0 && column > 0,
        "an API diagnostic has a zero line or column"
    );
    let mut column = column - 1;
    for insertion in instrumentation
        .insertions
        .get(&file)
        .into_iter()
        .flatten()
        .filter(|insertion| insertion.line == source_line)
    {
        if column >= insertion.column + insertion.bytes {
            column -= insertion.bytes;
        }
    }
    Ok(LineColumn {
        line: source_line,
        column,
    })
}

pub(super) fn validate_cargo_diagnostic_origin(
    diagnostic: &MarkerDiagnostic,
    inventory: &Inventory,
    root: &Path,
    file: usize,
    attributed_source: &str,
) -> Result<()> {
    let expected = &inventory.files[file].origin;
    let expected_source =
        std::fs::canonicalize(root.join(&expected.relative)).with_context(|| {
            format!(
                "canonicalizing expected Cargo target source {}",
                expected.relative.display()
            )
        })?;
    let reported_source =
        std::fs::canonicalize(&diagnostic.origin.target_source).with_context(|| {
            format!(
                "canonicalizing reported Cargo target source {}",
                diagnostic.origin.target_source.display()
            )
        })?;
    ensure!(
        diagnostic.origin.package_id == expected.package_id
            && diagnostic.origin.target_name == expected.name
            && diagnostic.origin.target_kind == expected.kind
            && reported_source == expected_source,
        "API diagnostic for {} at {} has Cargo origin {}:{}:{}:{}, expected {}:{}:{}:{}",
        inventory
            .nodes
            .get(diagnostic.id)
            .map_or("an unknown item", |node| node.label.as_str()),
        attributed_source,
        diagnostic.origin.package_id,
        diagnostic.origin.target_name,
        diagnostic.origin.target_kind.metadata_name(),
        reported_source.display(),
        expected.package_id,
        expected.name,
        expected.kind.metadata_name(),
        expected_source.display()
    );
    Ok(())
}

pub(super) fn cargo_diagnostic_origin_matches(
    diagnostic: &MarkerDiagnostic,
    inventory: &Inventory,
    root: &Path,
    file: usize,
) -> Result<bool> {
    let expected = &inventory.files[file].origin;
    let expected_source =
        std::fs::canonicalize(root.join(&expected.relative)).with_context(|| {
            format!(
                "canonicalizing expected Cargo target source {}",
                expected.relative.display()
            )
        })?;
    let reported_source =
        std::fs::canonicalize(&diagnostic.origin.target_source).with_context(|| {
            format!(
                "canonicalizing reported Cargo target source {}",
                diagnostic.origin.target_source.display()
            )
        })?;
    Ok(diagnostic.origin.package_id == expected.package_id
        && diagnostic.origin.target_name == expected.name
        && diagnostic.origin.target_kind == expected.kind
        && reported_source == expected_source)
}

pub(super) fn cargo_diagnostic_location(
    span: &BTreeMap<String, crate::json::JsonValue>,
    label: &str,
) -> Result<DiagnosticLocation> {
    let reported = crate::json::json_string(
        crate::json::json_field(span, "file_name", label)?,
        &format!("{label} `file_name`"),
    )?;
    let line = usize::try_from(crate::json::json_u64(
        crate::json::json_field(span, "line_start", label)?,
        &format!("{label} `line_start`"),
    )?)
    .with_context(|| format!("{label} line does not fit usize"))?;
    let column = usize::try_from(crate::json::json_u64(
        crate::json::json_field(span, "column_start", label)?,
        &format!("{label} `column_start`"),
    )?)
    .with_context(|| format!("{label} column does not fit usize"))?;
    ensure!(line > 0 && column > 0, "{label} has a zero line or column");
    Ok(DiagnosticLocation {
        reported: reported.to_string(),
        line,
        column,
    })
}

pub(super) fn diagnostic_marker_id(message: &str) -> Result<Option<usize>> {
    let markers = message.match_indices(MARKER).collect::<Vec<_>>();
    let [] = markers.as_slice() else {
        ensure!(
            markers.len() == 1,
            "compiler diagnostic contains {} API markers",
            markers.len()
        );
        let start = markers[0].0 + MARKER.len();
        let digits = message[start..]
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>();
        ensure!(
            !digits.is_empty(),
            "compiler diagnostic has a malformed API marker"
        );
        return Ok(Some(
            <usize as core::str::FromStr>::from_str(&digits)
                .context("compiler diagnostic API marker does not fit usize")?,
        ));
    };
    Ok(None)
}
