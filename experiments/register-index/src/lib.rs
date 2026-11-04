//! Experimental reading aid for the foundation register.
//!
//! `docs/016-pre-wizzy-foundation.md` is the authority.

use std::path::{Path, PathBuf};

/// Register states that still name kernel work.
pub const LIVE_STATES: [&str; 4] = ["REQUIRED", "READY", "VERIFY", "DECIDE"];

/// One parsed register row. Cells after the summary are omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `PW-` or `WZ-` identifier.
    pub id: String,
    /// State word from the register vocabulary.
    pub state: String,
    /// Priority cell, unchanged.
    pub priority: String,
    /// The "why this is live" cell, which stays meaningful after a row closes.
    pub summary: String,
}

/// Which rows a report includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// `REQUIRED`, `READY`, `VERIFY`, and `DECIDE`.
    Live,
    /// `TRIGGER` and `WIZZY`.
    Stopped,
    /// Live rows, then stopped rows.
    Assignment,
    /// Every parsed row.
    All,
}

/// Arguments for one invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// Register file. `None` means search upward from the working directory.
    pub register: Option<PathBuf>,
    /// Which rows to print.
    pub selection: Selection,
}

/// Why an invocation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError {
    message: String,
}

impl UsageError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for UsageError {}

/// Parse `PW-*` and `WZ-*` table rows in document order.
pub fn rows(document: &str) -> Vec<Row> {
    document.lines().filter_map(parse_row).collect()
}

/// `true` when the state still names live kernel work.
pub fn is_live(state: &str) -> bool {
    LIVE_STATES.contains(&state)
}

/// `true` for stopped compiler-gate and trigger rows.
pub fn is_stopped(state: &str) -> bool {
    state == "TRIGGER" || state == "WIZZY"
}

/// Identifiers that appear more than once.
pub fn duplicate_ids(parsed: &[Row]) -> Vec<&str> {
    let mut seen = Vec::<&str>::new();
    let mut duplicates = Vec::new();
    for row in parsed {
        if seen.contains(&row.id.as_str()) && !duplicates.contains(&row.id.as_str()) {
            duplicates.push(row.id.as_str());
        }
        seen.push(row.id.as_str());
    }
    duplicates
}

/// Rows selected for a report, in document order within each group.
pub fn select<'a>(parsed: &'a [Row], selection: Selection) -> Vec<&'a Row> {
    match selection {
        Selection::Live => parsed.iter().filter(|row| is_live(&row.state)).collect(),
        Selection::Stopped => parsed.iter().filter(|row| is_stopped(&row.state)).collect(),
        Selection::Assignment => {
            let mut chosen = select(parsed, Selection::Live);
            chosen.extend(select(parsed, Selection::Stopped));
            chosen
        }
        Selection::All => parsed.iter().collect(),
    }
}

/// Find `docs/016-pre-wizzy-foundation.md` at `start` or in a parent directory.
pub fn find_register(start: &Path) -> Option<PathBuf> {
    let mut directory = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(current) = directory {
        let candidate = current.join("docs").join("016-pre-wizzy-foundation.md");
        if candidate.is_file() {
            return Some(candidate);
        }
        directory = current.parent();
    }
    None
}

/// Parse process arguments after the program name.
pub fn invocation(args: &[String]) -> Result<Invocation, UsageError> {
    let mut register = None;
    let mut selection = Selection::Assignment;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--live" => selection = Selection::Live,
            "--stopped" => selection = Selection::Stopped,
            "--all" => selection = Selection::All,
            "--register" => {
                let path = arguments.next().ok_or_else(|| {
                    UsageError::new("`--register` needs the path of the foundation register")
                })?;
                register = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                return Err(UsageError::new(
                    "register-index [--register PATH] [--live | --stopped | --all]\n\
                     Prints rows from docs/016-pre-wizzy-foundation.md.\n\
                     The register remains authoritative; this tool is experimental.",
                ));
            }
            other => {
                return Err(UsageError::new(format!(
                    "unrecognised argument `{other}`; try --help"
                )));
            }
        }
    }
    Ok(Invocation {
        register,
        selection,
    })
}

/// Render selected rows. The summary is shortened so one row stays one line.
pub fn render(parsed: &[Row], selection: Selection) -> String {
    let chosen = select(parsed, selection);
    let mut output = format!(
        "rows {}  live {}  stopped {}  shown {}\n",
        parsed.len(),
        parsed.iter().filter(|row| is_live(&row.state)).count(),
        parsed.iter().filter(|row| is_stopped(&row.state)).count(),
        chosen.len()
    );
    for row in chosen {
        output.push_str(&row.id);
        output.push('\t');
        output.push_str(&row.state);
        output.push('\t');
        output.push_str(&row.priority);
        output.push('\t');
        output.push_str(&shorten(&row.summary, 120));
        output.push('\n');
    }
    output
}

fn parse_row(line: &str) -> Option<Row> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return None;
    }
    let mut cells = trimmed.trim_matches('|').split('|').map(str::trim);
    let id = cells.next()?;
    if !is_register_id(id) {
        return None;
    }
    let state = cells.next()?.to_string();
    let priority = cells.next().unwrap_or("").to_string();
    let summary = cells.next().unwrap_or("").to_string();
    Some(Row {
        id: id.to_string(),
        state,
        priority,
        summary,
    })
}

fn is_register_id(cell: &str) -> bool {
    let Some(rest) = cell
        .strip_prefix("PW-")
        .or_else(|| cell.strip_prefix("WZ-"))
    else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
}

fn shorten(text: &str, limit: usize) -> String {
    let mut shortened = String::new();
    for character in text.chars() {
        if shortened.chars().count() == limit {
            shortened.push('…');
            break;
        }
        shortened.push(character);
    }
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
| ID | State | Priority | Why this is live |\n\
| --- | --- | --- | --- |\n\
| PW-001 | DONE | P0 | qualified sweep |\n\
| PW-006 | REQUIRED | P1 | hostile-surface audit |\n\
| PW-010 | READY | P1 | census drift |\n\
| PW-601 | TRIGGER | WHEN-TRIGGERED | pinned artifact |\n\
| WZ-001 | WIZZY | GATE | external compiler |\n\
| not a row |\n\
";

    #[test]
    fn sample_rows_keep_document_order_and_skip_prose() {
        let parsed = rows(SAMPLE);
        let ids: Vec<_> = parsed.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["PW-001", "PW-006", "PW-010", "PW-601", "WZ-001"]);
        assert!(duplicate_ids(&parsed).is_empty());
    }

    #[test]
    fn assignment_lists_live_work_before_stopped_work() {
        let parsed = rows(SAMPLE);
        let shown: Vec<_> = select(&parsed, Selection::Assignment)
            .into_iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(shown, ["PW-006", "PW-010", "PW-601", "WZ-001"]);
        assert_eq!(
            select(&parsed, Selection::Live)
                .into_iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["PW-006", "PW-010"]
        );
    }

    #[test]
    fn invocation_accepts_one_selection_and_a_register_path() {
        let parsed = invocation(&[
            "--register".to_string(),
            "docs/016-pre-wizzy-foundation.md".to_string(),
            "--live".to_string(),
        ])
        .expect("arguments");
        assert_eq!(
            parsed.register.as_deref(),
            Some(Path::new("docs/016-pre-wizzy-foundation.md"))
        );
        assert_eq!(parsed.selection, Selection::Live);
        assert!(invocation(&["--register".to_string()]).is_err());
        assert!(invocation(&["--fast".to_string()]).is_err());
    }

    #[test]
    fn the_current_register_still_has_the_two_v0_2_0_live_rows() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("docs")
            .join("016-pre-wizzy-foundation.md");
        let document = std::fs::read_to_string(&path).expect("foundation register");
        let parsed = rows(&document);
        assert!(
            duplicate_ids(&parsed).is_empty(),
            "duplicate register ids: {:?}",
            duplicate_ids(&parsed)
        );
        let live: Vec<_> = select(&parsed, Selection::Live)
            .into_iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(
            live,
            ["PW-006", "PW-010"],
            "v0.2.0 left these live rows; update the experiment when the register changes"
        );
        let state = |id: &str| {
            parsed
                .iter()
                .find(|row| row.id == id)
                .map(|row| row.state.as_str())
        };
        assert_eq!(state("PW-001"), Some("DONE"));
        assert_eq!(state("PW-601"), Some("TRIGGER"));
        assert_eq!(state("WZ-001"), Some("WIZZY"));
        assert!(select(&parsed, Selection::Stopped).len() > 4);
    }
}
