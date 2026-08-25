//! What the documents say about the tree, checked against the tree.
//!
//! `docs/` is where this project's decisions live, and roughly every page
//! names crates by path. Nothing looked at those names, and the Phase 0–6
//! sweep found what that costs: `libs/xstate` was named three times in D-82 as
//! the crate holding the compacted-offset arithmetic, and never written — the
//! arithmetic turned out to be four instructions and a CPUID read, so there
//! was nothing for a crate to hold. A reader following the ADR finds nothing,
//! and the ADR reads like a description of the tree.
//!
//! **A crate named and not built is not automatically an error.** Research
//! notes propose crates that later phases will write, and an ADR that records
//! a crate being *declined* has to name it to decline it. So the check is not
//! "every name exists" but "every name that does not exist is on this list,
//! with why" — which turns an accident into a reviewable line in a commit, the
//! same mechanism the unsafe budget uses (D-124).

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};

/// A `libs/` crate the documents name and the tree does not have.
#[derive(Debug, Clone, Copy)]
pub struct Absent {
    /// The directory name it would have under `libs/`.
    pub crate_name: &'static str,
    /// Why it is named without existing.
    pub because: &'static str,
}

/// Every crate the documents may name without the tree holding it.
///
/// Adding a line here is the reviewable act. Removing one when the crate gets
/// written is the other half, and the check requires that too: an exemption
/// nobody removes is an exemption nobody reads.
pub const ABSENT: &[Absent] = &[
    Absent {
        crate_name: "xstate",
        because: "D-82 planned it; the arithmetic turned out to be four instructions and a \
                  CPUID read, which live in wost. The ADR says so at each of its three mentions.",
    },
    Absent {
        crate_name: "lock-rank",
        because: "D-85 declined it: a rank with one non-trivial pair is a type-level apparatus \
                  for a fact a comment states. All four mentions say it was not written.",
    },
    Absent {
        crate_name: "x86-page-tables",
        because: "`008`'s proposed name for what was built as `x86-paging`. A research note \
                  records what was proposed, so the name stays with a pointer beside it.",
    },
    Absent {
        crate_name: "wizfs-format",
        because: "`002` and `010`: the on-disk format crate, owed before WizFS's own phase \
                  (D-73). Not yet written and not yet due.",
    },
    Absent {
        crate_name: "blockdev-sim",
        because: "`010` §11: a fault-injecting block device for testing WizFS's crash \
                  consistency. Belongs to that phase.",
    },
];

/// Every `libs/<name>` the documents mention.
///
/// # Errors
///
/// If `docs/` cannot be read.
pub fn named_in_docs(root: &Path) -> Result<BTreeSet<String>> {
    let docs = root.join("docs");
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(&docs).with_context(|| format!("reading {}", docs.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        found.extend(mentions(&text));
    }
    Ok(found)
}

/// The crate names in `libs/<name>` mentions, in order of appearance.
fn mentions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("libs/") {
        let after = &rest[at + "libs/".len()..];
        let name: String = after
            .chars()
            .take_while(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || *char == '-')
            .collect();
        // A trailing hyphen means the scan stopped mid-word on punctuation
        // rather than at the end of a name.
        if !name.is_empty() && !name.ends_with('-') {
            found.push(name);
        }
        rest = after;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
            .to_path_buf()
    }

    /// The crates `libs/` actually holds.
    fn built(root: &Path) -> BTreeSet<String> {
        std::fs::read_dir(root.join("libs"))
            .expect("libs")
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                if !path.is_dir() {
                    return None;
                }
                Some(path.file_name()?.to_str()?.to_string())
            })
            .collect()
    }

    /// Every crate the documents name either exists or is declared absent.
    #[test]
    fn every_crate_the_documents_name_exists_or_is_declared_absent() {
        let root = root();
        let built = built(&root);
        let declared: BTreeSet<&str> = ABSENT.iter().map(|absent| absent.crate_name).collect();

        let unaccounted: Vec<String> = named_in_docs(&root)
            .expect("reading docs")
            .into_iter()
            .filter(|name| !built.contains(name) && !declared.contains(name.as_str()))
            .collect();

        assert!(
            unaccounted.is_empty(),
            "the documents name {unaccounted:?}, which the tree does not have and `ABSENT` does \
             not explain; write the crate, fix the name, or add a line saying why it is named \
             without existing"
        );
    }

    /// No declared absence names a crate that now exists.
    #[test]
    fn no_declared_absence_is_stale() {
        let root = root();
        let built = built(&root);
        for absent in ABSENT {
            assert!(
                !built.contains(absent.crate_name),
                "`libs/{}` exists now; remove its line from `ABSENT`",
                absent.crate_name
            );
            // A label is not a reason, and this list is only worth having if
            // each line says enough to judge it by.
            assert!(
                absent.because.len() > 40,
                "`libs/{}` needs a reason, not a label",
                absent.crate_name
            );
        }
    }

    /// The README's count of `ci`'s boots is `ci`'s count of its boots.
    ///
    /// It said three and ran four for as long as `degraded-check` has existed.
    /// A reader deciding whether the gate covers the no-x2APIC path reads that
    /// sentence, and it was wrong — which is the same defect class as a stale
    /// constant, in the file most likely to be someone's first (D-124).
    ///
    /// Counted from `tasks.rs`'s text rather than by running anything: the
    /// question is what `ci` is composed of, which is a property of the source.
    #[test]
    fn the_readme_counts_the_boots_that_ci_runs() {
        /// How the README spells each number it might say.
        const SPELLED: [(&str, usize); 5] = [
            ("two boots", 2),
            ("three boots", 3),
            ("four boots", 4),
            ("five boots", 5),
            ("six boots", 6),
        ];

        let root = root();
        let tasks = std::fs::read_to_string(root.join("xtask/src/tasks.rs")).expect("tasks.rs");
        let body = tasks
            .split_once("pub fn ci(paths: &Paths) -> Result<()> {")
            .expect("ci's definition")
            .1
            .split_once("\n}")
            .expect("ci's end")
            .0;
        let boots = body
            .lines()
            .filter(|line| {
                let line = line.trim();
                !line.starts_with("//")
                    && (line.starts_with("test_boot(")
                        || line.starts_with("boot_check(")
                        || line.starts_with("degraded_check("))
            })
            .count();
        assert!(
            boots >= 2,
            "found {boots} boots in `ci`; the scan is broken"
        );

        let readme = std::fs::read_to_string(root.join("README.md")).expect("README.md");
        let said = SPELLED
            .iter()
            .find(|(phrase, _)| readme.contains(phrase))
            .unwrap_or_else(|| panic!("the README does not say how many boots `ci` runs"));
        assert_eq!(
            said.1, boots,
            "the README says `{}` and `ci` runs {boots}",
            said.0
        );
    }

    #[test]
    fn a_mention_is_recognised_and_its_punctuation_is_not() {
        assert_eq!(mentions("see `libs/eevdf` for it"), vec!["eevdf"]);
        assert_eq!(mentions("libs/x86-apic/src/lib.rs"), vec!["x86-apic"]);
        assert_eq!(mentions("`libs/a`, `libs/b`."), vec!["a", "b"]);
        assert!(mentions("nothing here").is_empty());
    }
}
