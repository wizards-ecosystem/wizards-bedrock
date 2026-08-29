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
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
            .to_path_buf()
    }

    /// Project-authored Markdown whose paths and commands are promises to a
    /// reader. The bootloader README is upstream material and stays outside
    /// this list.
    fn markdown_documents(root: &Path) -> Vec<PathBuf> {
        let mut documents = [
            "README.md",
            "AGENTS.md",
            "userland/README.md",
            "tools/tla/README.md",
            "tools/fat32-corpus/README.md",
        ]
        .iter()
        .map(|relative| root.join(relative))
        .collect::<Vec<_>>();
        documents.extend(
            std::fs::read_dir(root.join("docs"))
                .expect("docs")
                .filter_map(|entry| {
                    let path = entry.ok()?.path();
                    (path.extension().and_then(|extension| extension.to_str()) == Some("md"))
                        .then_some(path)
                }),
        );
        documents.sort();
        documents
    }

    fn markdown_destinations(document: &str) -> Vec<(usize, String)> {
        let mut destinations = Vec::new();
        for (line_index, line) in document.lines().enumerate() {
            let mut rest = line;
            while let Some(open) = rest.find("](") {
                let after = &rest[open + 2..];
                let Some(close) = after.find(')') else {
                    break;
                };
                let raw = after[..close]
                    .split_ascii_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_matches(['<', '>']);
                if !raw.is_empty() {
                    destinations.push((line_index + 1, raw.to_string()));
                }
                rest = &after[close + 1..];
            }
        }
        destinations
    }

    fn heading_anchors(document: &str) -> BTreeSet<String> {
        let mut anchors = BTreeSet::new();
        let mut repetitions = BTreeMap::<String, usize>::new();
        for line in document.lines() {
            let line = line.trim_start();
            let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
            if !(1..=6).contains(&hashes)
                || !line[hashes..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
            {
                continue;
            }
            let heading = line[hashes..].trim().trim_end_matches('#').trim();
            let mut slug = String::new();
            for character in heading.chars().flat_map(char::to_lowercase) {
                if character.is_alphanumeric() || character == '_' || character == '-' {
                    slug.push(character);
                } else if character.is_whitespace() {
                    slug.push('-');
                }
            }
            let repeated = repetitions.entry(slug.clone()).or_default();
            let anchor = if *repeated == 0 {
                slug.clone()
            } else {
                format!("{slug}-{repeated}")
            };
            *repeated += 1;
            anchors.insert(anchor);
        }
        anchors
    }

    fn local_markdown_failures(root: &Path, source: &Path, document: &str) -> Vec<String> {
        let mut failures = Vec::new();
        let workspace = root.canonicalize().expect("workspace root");
        for (line, destination) in markdown_destinations(document) {
            if destination.contains("://") || destination.starts_with("mailto:") {
                continue;
            }
            let (path, fragment) = destination
                .split_once('#')
                .map_or((destination.as_str(), None), |(path, fragment)| {
                    (path, Some(fragment))
                });
            let target = if path.is_empty() {
                source.to_path_buf()
            } else if let Some(path) = path.strip_prefix('/') {
                root.join(path)
            } else {
                source
                    .parent()
                    .expect("Markdown source has a parent")
                    .join(path)
            };
            let label = source.strip_prefix(root).unwrap_or(source).display();
            if !target.exists() {
                failures.push(format!(
                    "{label}:{line}: local Markdown target `{destination}` does not exist"
                ));
                continue;
            }
            let resolved = target
                .canonicalize()
                .unwrap_or_else(|_| panic!("resolving {}", target.display()));
            if !resolved.starts_with(&workspace) {
                failures.push(format!(
                    "{label}:{line}: local Markdown target `{destination}` leaves the workspace"
                ));
                continue;
            }
            let Some(fragment) = fragment else {
                continue;
            };
            if fragment.is_empty() || !resolved.is_file() {
                failures.push(format!(
                    "{label}:{line}: local Markdown anchor `{destination}` has no document heading"
                ));
                continue;
            }
            let target_document = std::fs::read_to_string(&resolved)
                .unwrap_or_else(|_| panic!("reading {}", resolved.display()));
            if !heading_anchors(&target_document).contains(fragment) {
                failures.push(format!(
                    "{label}:{line}: local Markdown anchor `{destination}` does not resolve"
                ));
            }
        }
        failures
    }

    fn xtask_invocations(document: &str) -> Vec<(usize, Vec<OsString>)> {
        let mut invocations = Vec::new();
        let mut fragments = Vec::new();
        let mut cursor = 0usize;
        while let Some(relative) = document[cursor..].find('`') {
            let open = cursor + relative;
            let ticks = document[open..]
                .bytes()
                .take_while(|byte| *byte == b'`')
                .count();
            cursor = open + ticks;
            if ticks != 1 {
                continue;
            }
            let Some(relative_close) = document[cursor..].find('`') else {
                break;
            };
            let close = cursor + relative_close;
            fragments.push((open, &document[cursor..close]));
            cursor = close + 1;
        }

        let mut in_fence = false;
        let mut line_offset = 0usize;
        for line in document.split_inclusive('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                in_fence = !in_fence;
            } else if in_fence && trimmed.starts_with("cargo xtask") {
                fragments.push((line_offset, trimmed));
            }
            line_offset += line.len();
        }

        for (offset, fragment) in fragments {
            let Some(at) = fragment.find("cargo xtask") else {
                continue;
            };
            let after = &fragment[at + "cargo xtask".len()..];
            let args = after
                .split_ascii_whitespace()
                .take_while(|argument| {
                    argument.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "-_=.".contains(character)
                    })
                })
                .map(OsString::from)
                .collect::<Vec<_>>();
            if args.is_empty() {
                continue;
            }
            let line = document[..offset]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            invocations.push((line, args));
        }
        invocations
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

    /// The phase's closing table says what the source says.
    ///
    /// `014`'s per-slice lines are history — what the gate reported the day
    /// that slice landed — and seven of them were wrong, written from
    /// expectation before the gate had run. History cannot be checked against
    /// the present, but the **closing table** is a claim about the tree as it
    /// stands, and that can be (D-175).
    ///
    /// The same mechanism as the README's boot count, and the same reason
    /// `002` gives for pointing at the exact sysdep register rather than
    /// restating it: a claim copied to a second place can disagree with itself,
    /// and the copy that is never read is the one that goes stale.
    #[test]
    fn the_phase_close_says_what_the_source_says() {
        let root = root();
        let storage =
            std::fs::read_to_string(root.join("docs/014-storage.md")).expect("014-storage.md");
        let selftest =
            std::fs::read_to_string(root.join("kernel/src/selftest.rs")).expect("selftest.rs");

        // `| Machine checks | 180 |`
        let claimed_checks = table_number(&storage, "Machine checks");
        let expected = selftest
            .lines()
            .find_map(|line| line.trim().strip_prefix("const EXPECTED_CHECKS: usize = "))
            .and_then(|value| value.trim_end_matches(';').parse::<usize>().ok())
            .expect("no EXPECTED_CHECKS in selftest.rs");
        assert_eq!(
            claimed_checks, expected,
            "`014`'s closing table says {claimed_checks} machine checks and the kernel expects \
             {expected}"
        );

        // `| Mutations | 64, each caught ... |`
        let claimed_mutations = table_number(&storage, "Mutations");
        assert_eq!(
            claimed_mutations,
            crate::mutate::MUTATIONS.len(),
            "`014`'s closing table says {claimed_mutations} mutations and there are {}",
            crate::mutate::MUTATIONS.len()
        );

        let mutation_numbers = table_numbers(&storage, "Mutations");
        assert_eq!(
            mutation_numbers.len(),
            3,
            "the mutation row must state total, last full sweep and later individual count"
        );
        assert_eq!(
            mutation_numbers[0] - mutation_numbers[1],
            mutation_numbers[2],
            "`014`'s later-mutation count is not total minus the last full sweep"
        );

        let decisions =
            std::fs::read_to_string(root.join("docs/004-decision-log.md")).expect("decision log");
        let latest_decision = decisions
            .lines()
            .filter_map(|line| line.strip_prefix("## D-"))
            .filter_map(|rest| rest.split_ascii_whitespace().next())
            .filter_map(|number| number.parse::<usize>().ok())
            .max()
            .expect("a decision heading");
        let claimed_decision = table_numbers(&storage, "Decisions")
            .into_iter()
            .max()
            .expect("a decision number in `014`");
        assert_eq!(
            claimed_decision, latest_decision,
            "`014` ends at D-{claimed_decision}, but the log ends at D-{latest_decision}"
        );
    }

    /// Current summaries name later resolutions without rewriting old evidence.
    #[test]
    fn current_facing_foundation_prose_names_the_later_resolutions() {
        let root = root();
        let read = |path: &str| {
            std::fs::read_to_string(root.join(path)).unwrap_or_else(|_| panic!("reading {path}"))
        };
        let vision = read("docs/000-vision.md");
        let boot = read("docs/001-boot.md");
        let multitasking = read("docs/009-multitasking.md");
        let userspace = read("docs/011-userspace.md");
        let research = read("docs/012-research-notes-4.md");
        let decisions = read("docs/004-decision-log.md");
        let ports = read("wost/src/arch/x86_64/port.rs");

        for decision in ["D-121", "D-125", "D-154", "D-164", "D-189", "D-214"] {
            assert!(
                userspace.contains(decision),
                "the current Phase 5 summary omits {decision}'s later resolution"
            );
        }
        for decision in ["D-89", "D-92", "D-164"] {
            assert!(
                multitasking.contains(decision),
                "the current Phase 4 summary omits {decision}'s later resolution"
            );
        }
        assert!(
            userspace.contains("A first-class `MemoryObject` does not"),
            "Phase 5 prose can be read as claiming a first-class MemoryObject"
        );
        assert!(
            userspace.contains("That is a ceiling, not a dynamic-stack implementation"),
            "Phase 5 prose can be read as claiming dynamic stacks"
        );
        assert!(
            !userspace.contains("`SPINLOCKS_HELD` is a global"),
            "Phase 5 prose restores the pre-D-154 global-lock-count claim"
        );
        assert!(
            boot.contains("the module carrying the initrd"),
            "the boot contract still presents the shipped initrd as future"
        );
        assert!(
            research.contains("Resolved by D-162"),
            "the research summary still presents the anchored backslash code as open"
        );
        assert!(
            ports.contains("[`claim_pair`] answers that for i8042"),
            "the port contract does not name i8042's atomic pair"
        );
        assert!(
            vision.contains("an own bootloader is not planned without a concrete unmet handoff requirement (D-219)"),
            "the roadmap reschedules the declined own-bootloader project"
        );
        assert!(
            decisions.contains("**Amended by D-219.** Limine remains the boot boundary"),
            "D-03 does not name its current bootloader amendment"
        );

        assert!(
            vision.contains("124 machine checks at the close")
                && vision.contains("both have moved since"),
            "the dated Phase 5 count is no longer explicitly historical"
        );
        assert!(
            userspace.contains("### What the phase cost, measured")
                && userspace.contains("**124 machine checks**, from 79 at the Phase 4 close"),
            "the Phase 5 evidence totals lost their historical context"
        );
    }

    /// The first number in the table row whose left cell is `label`.
    fn table_number(document: &str, label: &str) -> usize {
        table_numbers(document, label)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("the `{label}` row states no number"))
    }

    fn table_numbers(document: &str, label: &str) -> Vec<usize> {
        let row = document
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("| {label} |")))
            .unwrap_or_else(|| panic!("no `{label}` row in the closing table"));
        row.split('|')
            .nth(2)
            .into_iter()
            .flat_map(|cell| {
                cell.split(|character: char| !character.is_ascii_digit())
                    .filter(|digits| !digits.is_empty())
                    .filter_map(|digits| digits.parse().ok())
            })
            .collect()
    }

    #[test]
    fn every_local_markdown_link_and_anchor_resolves() {
        let root = root();
        let mut failures = Vec::new();
        for source in markdown_documents(&root) {
            let document = std::fs::read_to_string(&source)
                .unwrap_or_else(|_| panic!("reading {}", source.display()));
            failures.extend(local_markdown_failures(&root, &source, &document));
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn every_documented_xtask_invocation_parses() {
        let root = root();
        let mut failures = Vec::new();
        for source in markdown_documents(&root) {
            let document = std::fs::read_to_string(&source)
                .unwrap_or_else(|_| panic!("reading {}", source.display()));
            let label = source.strip_prefix(&root).unwrap_or(&source).display();
            for (line, args) in xtask_invocations(&document) {
                if let Err(error) = crate::cli::parse(args) {
                    failures.push(format!("{label}:{line}: {error}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
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

    /// Current-facing plans cannot quietly turn D-183 back into an experiment.
    #[test]
    fn wizzy_is_the_first_party_target_and_its_compiler_gate_is_closed() {
        const CURRENT_PLANS: [&str; 7] = [
            "AGENTS.md",
            "README.md",
            "docs/000-vision.md",
            "docs/002-architecture.md",
            "docs/013-application-platform.md",
            "docs/015-wizzy-userland.md",
            "docs/016-pre-wizzy-foundation.md",
        ];
        const RETIRED_WORDING: [&str; 5] = [
            "provisionally Wizzy",
            "if D-180's promotion gate passes",
            "revisits the Wizzy selection",
            "fallback if the Wizzy gate fails",
            "compiled Rust boundary probe",
        ];

        let root = root();
        let policy = std::fs::read_to_string(root.join("docs/015-wizzy-userland.md"))
            .expect("015-wizzy-userland.md");
        let table_value = |label: &str| {
            policy
                .lines()
                .find(|line| line.starts_with(&format!("| {label} |")))
                .and_then(|line| line.split('|').nth(2))
                .map(str::trim)
                .unwrap_or_else(|| panic!("the Wizzy policy has no `{label}` row"))
        };

        assert_eq!(table_value("First-party target language"), "Wizzy");
        assert_eq!(table_value("Compiler-readiness gate"), "**CLOSED**");
        assert!(
            policy.contains("Phase 7 product implementation is stopped"),
            "a closed compiler gate no longer stops Phase 7 product work"
        );

        for path in CURRENT_PLANS {
            let document =
                std::fs::read_to_string(root.join(path)).unwrap_or_else(|_| panic!("{path}"));
            assert!(
                document.contains("D-183"),
                "{path} does not point at the governing Wizzy-first decision"
            );
            for retired in RETIRED_WORDING {
                assert!(
                    !document.contains(retired),
                    "{path} restores D-180's retired `{retired}` language-selection wording"
                );
            }
        }
    }

    /// Closed-gate work is a finite register, not an unbounded polish label.
    #[test]
    fn the_pre_wizzy_register_is_finite_and_evidence_bound() {
        const STATES: [&str; 8] = [
            "REQUIRED",
            "READY",
            "VERIFY",
            "DECIDE",
            "WIZZY",
            "TRIGGER",
            "DONE",
            "SUPERSEDED",
        ];
        const PRIORITIES: [&str; 5] = ["P0", "P1", "P2", "GATE", "WHEN-TRIGGERED"];

        let root = root();
        let register = std::fs::read_to_string(root.join("docs/016-pre-wizzy-foundation.md"))
            .expect("016-pre-wizzy-foundation.md");
        let (_, body) = register
            .split_once("<!-- PRE-WIZZY-REGISTER:START -->")
            .expect("pre-Wizzy register start marker");
        let (body, _) = body
            .split_once("<!-- PRE-WIZZY-REGISTER:END -->")
            .expect("pre-Wizzy register end marker");
        let mut ids = BTreeSet::new();
        let mut rows = 0usize;

        for line in body.lines().filter(|line| {
            let line = line.trim_start();
            line.starts_with("| PW-") || line.starts_with("| WZ-")
        }) {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            assert_eq!(cells.len(), 6, "register row has the wrong shape: {line}");
            assert!(ids.insert(cells[0]), "duplicate register ID `{}`", cells[0]);
            assert!(STATES.contains(&cells[1]), "unknown state in {line}");
            assert!(PRIORITIES.contains(&cells[2]), "unknown priority in {line}");
            assert!(
                cells[3].len() > 35,
                "{} has no substantive reason",
                cells[0]
            );
            assert!(
                cells[4].len() > 25,
                "{} has no finite deliverable",
                cells[0]
            );
            assert!(cells[5].len() > 35, "{} has no close evidence", cells[0]);
            rows += 1;
        }

        assert!(rows >= 40, "the register scan found only {rows} rows");
        for required in ["PW-001", "PW-100", "PW-200", "PW-300", "WZ-001"] {
            assert!(
                ids.contains(required),
                "the register lost anchor {required}"
            );
        }
        assert!(
            register.contains("D-184"),
            "the register does not name its decision"
        );

        for path in [
            "AGENTS.md",
            "README.md",
            "docs/000-vision.md",
            "docs/002-architecture.md",
            "docs/013-application-platform.md",
            "docs/015-wizzy-userland.md",
        ] {
            let document =
                std::fs::read_to_string(root.join(path)).unwrap_or_else(|_| panic!("{path}"));
            assert!(
                document.contains("D-184"),
                "{path} does not point at the finite register"
            );
            assert!(
                document.contains("016-pre-wizzy-foundation.md"),
                "{path} does not point at the finite register document"
            );
        }
    }

    #[test]
    fn the_ci_registry_drives_help_and_documentation() {
        let root = root();
        crate::gate::verify_documentation(&root).expect("README renders the CI registry");

        let help = crate::cli::help();
        for pass in crate::gate::REGISTRY.cargo_passes() {
            assert!(help.contains(&pass.label()), "help omits {}", pass.label());
        }
        for step in crate::gate::REGISTRY.steps_for_test() {
            assert!(
                help.contains(step.invocation()),
                "help omits {}",
                step.invocation()
            );
        }
    }

    #[test]
    fn a_mention_is_recognised_and_its_punctuation_is_not() {
        assert_eq!(mentions("see `libs/eevdf` for it"), vec!["eevdf"]);
        assert_eq!(mentions("libs/x86-apic/src/lib.rs"), vec!["x86-apic"]);
        assert_eq!(mentions("`libs/a`, `libs/b`."), vec!["a", "b"]);
        assert!(mentions("nothing here").is_empty());
    }
}
