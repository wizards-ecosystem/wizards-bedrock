use super::*;

/// The workspace root, from where this crate sits below it.
fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root")
        .to_path_buf()
}

/// The log numbers its entries from one upward with no gap and no repeat.
///
/// A gap is a decision that was minted and lost, or a number skipped by
/// accident; the log is append-only, so either is a mistake rather than a
/// style. The repeat half is not hypothetical either — two sessions
/// appending on the same day is exactly how it would happen.
#[test]
fn the_decisions_are_numbered_without_a_gap() {
    let numbers = defined(&root()).expect("reading the decision log");
    assert!(!numbers.is_empty(), "the log defines no decisions");

    let last = *numbers.last().expect("non-empty");
    let missing: Vec<u32> = (1..=last)
        .filter(|number| !numbers.contains(number))
        .collect();
    assert!(
        missing.is_empty(),
        "the log jumps over {missing:?}; a cited number with no entry is a rationale that leads nowhere"
    );

    // A repeat cannot survive the set above, so it is counted separately.
    let headings = entries(&root()).expect("reading the decision log").len();
    assert_eq!(
        headings,
        numbers.len(),
        "two entries share a number, or one heading is malformed"
    );
}

/// Every `D-NNN` written anywhere names an entry the log defines.
///
/// This is the one that would have caught D-121, cited in eight places
/// with nothing behind it.
#[test]
fn every_citation_names_a_decision_that_exists() {
    let root = root();
    let defined = defined(&root).expect("reading the decision log");
    let mut dangling = Vec::new();

    for path in searchable(&root).expect("walking the tree") {
        let text = searchable_text(&path);
        for (decision, line) in cited(&text) {
            if !defined.contains(&decision) {
                let shown = path.strip_prefix(&root).unwrap_or(&path);
                dangling.push(format!("{}:{line} cites D-{decision:02}", shown.display()));
            }
        }
    }

    assert!(
        dangling.is_empty(),
        "{} citation(s) name a decision the log does not define:\n  {}",
        dangling.len(),
        dangling.join("\n  ")
    );
}

const FIRST: (&str, &str) = (
    "0001-first.md",
    "# D-01 First\n\nStatus: Accepted\n\nBody.\n",
);
const SECOND: (&str, &str) = (
    "0002-second.md",
    "# D-2 Second\n\nStatus: Accepted, 2026-10-25.\n\nAmends D-01.\n",
);
const FIRST_ROW: &str = "| [D-01 First](decisions/0001-first.md) | Accepted | 2026-07-18 |";
const SECOND_ROW: &str = "| [D-2 Second](decisions/0002-second.md) | Accepted | 2026-10-25 |";

fn split_tree(files: &[(&str, &str)], rows: &[&str]) -> std::path::PathBuf {
    let root = crate::test_support::reserve_temp_directory("wizard-decisions");
    std::fs::create_dir_all(root.join(DIRECTORY)).expect("creating the decision directory");
    for (name, text) in files {
        std::fs::write(root.join(DIRECTORY).join(name), text).expect("writing a decision");
    }
    let index = format!(
        "# 004 - Decision log\n\n| Decision | Status | Date |\n|---|---|---|\n{}\n",
        rows.join("\n")
    );
    std::fs::write(root.join(LOG), index).expect("writing the index");
    root
}

fn refused(files: &[(&str, &str)], rows: &[&str]) -> bool {
    let root = split_tree(files, rows);
    let refused = defined(&root).is_err();
    std::fs::remove_dir_all(&root).expect("removing the decision fixture");
    refused
}

#[test]
fn the_split_layout_defines_one_decision_per_file() {
    let root = split_tree(&[FIRST, SECOND], &[FIRST_ROW, SECOND_ROW]);
    let numbers = defined(&root).expect("reading the split layout");
    let second = decision_text(&root, 2).expect("reading D-2");
    let count = entries(&root).expect("reading the split layout").len();
    std::fs::remove_dir_all(&root).expect("removing the decision fixture");
    assert_eq!(numbers, BTreeSet::from([1, 2]));
    assert_eq!(count, 2);
    assert_eq!(second, SECOND.1);
}

#[test]
fn an_index_that_disagrees_with_the_files_is_refused() {
    assert!(
        refused(&[FIRST, SECOND], &[SECOND_ROW, FIRST_ROW]),
        "out of order"
    );
    assert!(refused(&[FIRST, SECOND], &[FIRST_ROW]), "a file left out");
    assert!(
        refused(&[FIRST, SECOND], &[FIRST_ROW, SECOND_ROW, SECOND_ROW]),
        "a file listed twice"
    );
    assert!(
        refused(
            &[FIRST, SECOND],
            &[
                FIRST_ROW,
                "| [D-2 Other](decisions/0002-second.md) | Accepted | 2026-10-25 |"
            ]
        ),
        "a title that is not the file's"
    );
}

#[test]
fn a_decision_file_out_of_shape_is_refused() {
    assert!(
        !refused(&[FIRST], &[FIRST_ROW]),
        "the well-formed fixture is accepted"
    );
    assert!(
        refused(
            &[("0001-first.md", "# D-3 First\n\nStatus: Accepted\n")],
            &["| [D-3 First](decisions/0001-first.md) | Accepted | |"]
        ),
        "a heading that disagrees with its file name"
    );
    assert!(
        refused(
            &[("0001-first.md", "# D-01 First\n\nBody.\n")],
            &[FIRST_ROW]
        ),
        "no status line"
    );
    assert!(
        refused(
            &[("0001-first.txt", FIRST.1)],
            &["| [D-01 First](decisions/0001-first.txt) | Accepted | |"]
        ),
        "a decision that is not Markdown"
    );
}

#[test]
fn a_citation_is_recognised_and_a_lookalike_is_not() {
    assert_eq!(cited("as D-105 says"), vec![(105, 1)]);
    assert_eq!(cited("(D-12, D-100)"), vec![(12, 1), (100, 1)]);
    // Not citations: a boundary in front, a boundary behind, and digits
    // too long to be one.
    assert!(cited("MD-5 is a hash").is_empty());
    assert!(cited("D-0DEADBEEF").is_empty(), "hex, not a decision");
    assert!(cited("D-1234").is_empty(), "too long to be a decision");
    assert!(cited("D- alone").is_empty());
    // And the forms that are: mid-sentence, parenthesised, possessive.
    assert_eq!(cited("D-105's rule"), vec![(105, 1)]);
}

#[test]
fn an_unreadable_searchable_file_fails_the_citation_proof() {
    let missing = root().join("xtask/src/__missing_decision_proof_file__");
    assert!(
        !missing.exists(),
        "the missing-file fixture unexpectedly exists"
    );
    assert!(std::panic::catch_unwind(|| searchable_text(&missing)).is_err());
}
