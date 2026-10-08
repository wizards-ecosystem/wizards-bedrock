//! What the documents say about the tree, checked against the tree.
//!
//! Documentation may name planned or rejected crates, so missing paths are not
//! automatically errors. Every missing documented crate instead needs one
//! explicit exemption with its reason, making drift reviewable (D-124).

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
    for entry in crate::paths::directory_entries(&docs)? {
        let path = entry.path();
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
            "fixtures/abi-pressure/README.md",
            "tools/tla/README.md",
            "tools/fat32-corpus/README.md",
        ]
        .iter()
        .map(|relative| root.join(relative))
        .collect::<Vec<_>>();
        let paths = crate::paths::directory_entries(&root.join("docs"))
            .expect("complete docs directory listing")
            .into_iter()
            .map(|entry| entry.path());
        documents.extend(paths.into_iter().filter(|path| {
            path.extension().and_then(|extension| extension.to_str()) == Some("md")
        }));
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
                let raw = core::iter::Iterator::next(&mut after[..close].split_ascii_whitespace())
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
                || !core::iter::Iterator::next(&mut line[hashes..].chars())
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
        crate::paths::directory_entries(&root.join("libs"))
            .expect("complete libs directory listing")
            .into_iter()
            .filter(|entry| entry.is_dir())
            .map(|entry| entry.path())
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_else(|| panic!("library path is not UTF-8: {}", path.display()))
                    .to_string()
            })
            .collect()
    }

    /// `014`'s current closing table must match source; dated per-slice history
    /// remains historical rather than being rewritten (D-175).
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
            .and_then(|value| {
                <usize as core::str::FromStr>::from_str(value.trim_end_matches(';')).ok()
            })
            .expect("no EXPECTED_CHECKS in selftest.rs");
        assert_eq!(
            claimed_checks, expected,
            "`014`'s closing table says {claimed_checks} machine checks and the kernel expects \
             {expected}"
        );
        assert_eq!(
            current_numeric_claim(&storage),
            None,
            "`014` must keep its closing table as the sole current numeric summary"
        );

        // `| Mutations | 64 registered; ... |`
        let claimed_mutations = table_number(&storage, "Mutations");
        assert_eq!(
            claimed_mutations,
            crate::mutate::MUTATIONS.len(),
            "`014`'s closing table says {claimed_mutations} mutations and there are {}",
            crate::mutate::MUTATIONS.len()
        );

        let mutation_numbers = table_numbers(&storage, "Mutations");
        assert_eq!(
            mutation_numbers,
            [crate::mutate::MUTATIONS.len()],
            "the mutation row must state only its source-derived registry count"
        );
        let mutation_cell = table_cell(&storage, "Mutations");
        assert!(
            mutation_cell.contains("registered")
                && mutation_cell
                    .contains("complete-sweep verdicts live in source-bound qualification records")
                && ["caught", "surviv", "inconclusive"]
                    .into_iter()
                    .all(|claim| !mutation_cell.contains(claim)),
            "the mutation row makes mutable prose authoritative for sweep verdicts"
        );

        let decisions =
            std::fs::read_to_string(root.join("docs/004-decision-log.md")).expect("decision log");
        let latest_decision = decisions
            .lines()
            .filter_map(|line| line.strip_prefix("## D-"))
            .filter_map(|rest| core::iter::Iterator::next(&mut rest.split_ascii_whitespace()))
            .filter_map(|number| <usize as core::str::FromStr>::from_str(number).ok())
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

    /// Living documents cite mutations by permanent identity, so every cited
    /// identity must still name a registered mutation (D-341).
    #[test]
    fn every_living_mutation_citation_names_a_registered_identity() {
        let root = root();
        let registered = crate::mutate::MUTATIONS
            .iter()
            .map(|mutation| u64::from(mutation.id))
            .collect::<BTreeSet<_>>();
        let retired = crate::mutate::RETIRED_MUTATION_IDS
            .iter()
            .map(|id| u64::from(*id))
            .collect::<BTreeSet<_>>();
        let mut cited = 0;
        let mut unknown = Vec::new();
        for path in [
            "docs/016-pre-wizzy-foundation.md",
            "docs/017-pre-wizzy-hostile-surface-audit.md",
            "docs/014-storage.md",
            "README.md",
            "AGENTS.md",
        ] {
            let document = std::fs::read_to_string(root.join(path))
                .unwrap_or_else(|_| panic!("reading {path}"));
            let lines = document.lines().collect::<Vec<_>>();
            for (line, id) in mutation_citations(&document) {
                cited += 1;
                let retired_here = retired.contains(&id) && lines[line - 1].contains("retired");
                if !registered.contains(&id) && !retired_here {
                    unknown.push(format!("{path}:{line} #{id}"));
                }
            }
        }
        assert!(cited > 0, "no living document cites a mutation");
        assert!(
            unknown.is_empty(),
            "living documents cite unregistered mutations: {}",
            unknown.join(", ")
        );
    }

    /// The sweep guide's boot count is derived from the registry, so an
    /// operator budgeting machine time never reads an estimate.
    #[test]
    fn the_sweep_guide_states_the_exact_machine_witness_count() {
        use crate::mutate::Caught;

        let register = std::fs::read_to_string(root().join("docs/016-pre-wizzy-foundation.md"))
            .expect("016-pre-wizzy-foundation.md");
        let words = register.split_whitespace().collect::<Vec<_>>();
        let claims = words
            .windows(6)
            .filter(|window| window[1..] == ["of", "its", "entries", "boot", "QEMU,"])
            .map(|window| window[0])
            .collect::<Vec<_>>();
        let [claimed] = claims.as_slice() else {
            panic!(
                "the sweep guide states its QEMU boot count {} times",
                claims.len()
            );
        };
        let machine = crate::mutate::MUTATIONS
            .iter()
            .filter(|mutation| {
                !matches!(
                    mutation.reddens,
                    Caught::HostTest { .. } | Caught::ApiAudit(_)
                )
            })
            .count();
        assert_eq!(
            *claimed,
            machine.to_string(),
            "the sweep guide says {claimed} entries boot QEMU and the registry has {machine}"
        );
    }

    #[test]
    fn mutation_citations_read_every_list_and_range_form() {
        let ids = |text: &str| {
            mutation_citations(text)
                .into_iter()
                .map(|(_, id)| id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("mutation 17 failed"), [17]);
        assert_eq!(ids("Mutations 1,390 and 1,391 each"), [1_390, 1_391]);
        assert_eq!(
            ids("mutations 326, 327 and 1,311 through 1,313 red"),
            [326, 327, 1_311, 1_312, 1_313]
        );
        assert_eq!(ids("mutations 237-239 retain"), [237, 238, 239]);
        assert_eq!(ids("mutation 1219 retains"), [1_219]);
        assert_eq!(ids("mutation 17, and the rest"), [17]);
        assert_eq!(ids("mutations 9 through 7"), [9, u64::MAX]);
        assert!(ids("mutation-only recovery; the mutation sweep; 12 mutations").is_empty());
    }

    /// Every identity a citation such as `mutations 12, 14 and 1,020 through
    /// 1,022` names, with its one-based line. An inverted range yields
    /// `u64::MAX`, which no registry holds.
    fn mutation_citations(document: &str) -> Vec<(usize, u64)> {
        let mut cited = Vec::new();
        for (index, line) in document.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            let mut offset = 0;
            while let Some(found) = lower[offset..].find("mutation") {
                let start = offset + found;
                offset = start + "mutation".len();
                if lower[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|character| character.is_ascii_alphanumeric())
                {
                    continue;
                }
                let rest = &line[offset..];
                let rest = rest.strip_prefix('s').unwrap_or(rest);
                let Some((first, mut rest)) = rest.strip_prefix(' ').and_then(citation_number)
                else {
                    continue;
                };
                cited.push((index + 1, first));
                let mut previous = first;
                loop {
                    if let Some(after) = [" through ", "-"]
                        .into_iter()
                        .find_map(|separator| rest.strip_prefix(separator))
                    {
                        let Some((last, after)) = citation_number(after) else {
                            break;
                        };
                        if last < previous {
                            cited.push((index + 1, u64::MAX));
                        }
                        cited.extend((previous + 1..=last).map(|id| (index + 1, id)));
                        previous = last;
                        rest = after;
                    } else if let Some(after) = [", and ", ", ", " and "]
                        .into_iter()
                        .find_map(|separator| rest.strip_prefix(separator))
                    {
                        let Some((next, after)) = citation_number(after) else {
                            break;
                        };
                        cited.push((index + 1, next));
                        previous = next;
                        rest = after;
                    } else {
                        break;
                    }
                }
            }
        }
        cited
    }

    /// One decimal number, with or without thousands separators, and the text
    /// after it.
    fn citation_number(text: &str) -> Option<(u64, &str)> {
        let bytes = text.as_bytes();
        let mut end = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if end == 0 {
            return None;
        }
        while bytes.get(end) == Some(&b',')
            && bytes
                .get(end + 1..end + 4)
                .is_some_and(|group| group.iter().all(u8::is_ascii_digit))
            && !bytes.get(end + 4).is_some_and(u8::is_ascii_digit)
        {
            end += 4;
        }
        let value = <u64 as core::str::FromStr>::from_str(&text[..end].replace(',', "")).ok()?;
        Some((value, &text[end..]))
    }

    #[test]
    fn current_numeric_history_is_distinct_from_a_current_claim() {
        assert_eq!(
            current_numeric_claim("2026-08-25 had 189 checks; the then-current 189-check image"),
            None
        );
        assert_eq!(
            current_numeric_claim("the current **189**-check image"),
            Some(189)
        );
        assert_eq!(current_numeric_claim("concurrently 189 checks"), None);
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
        let readme = read("README.md");
        let instructions = read("AGENTS.md");

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
            instructions.contains("both a pinned draft\nartifact and a runtime seam"),
            "the contributor gate can authorize scaffolding without a runtime seam"
        );
        assert!(
            decisions.contains(
                "scaffolding remains stopped until both a pinned draft artifact and its runtime\nseam exist"
            ),
            "D-183 can be read as authorizing OS scaffolding before PW-601"
        );
        assert!(
            readme.contains("`cargo xtask qualify-vmware --shipped`"),
            "the phase-close guide omits shipped VMware transcript qualification"
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
        core::iter::Iterator::next(&mut table_numbers(document, label).into_iter())
            .unwrap_or_else(|| panic!("the `{label}` row states no number"))
    }

    fn current_numeric_claim(document: &str) -> Option<usize> {
        document.match_indices("current").find_map(|(offset, _)| {
            if document[..offset]
                .chars()
                .next_back()
                .is_some_and(|character| {
                    character.is_alphanumeric() || matches!(character, '_' | '-')
                })
            {
                return None;
            }
            let suffix = &document[offset + "current".len()..];
            if suffix
                .chars()
                .next()
                .is_some_and(|character| character.is_alphanumeric() || character == '_')
            {
                return None;
            }
            let suffix = suffix.trim_start_matches(|character: char| {
                character.is_whitespace() || matches!(character, '`' | '*' | '_')
            });
            let digits = suffix
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            (!digits.is_empty())
                .then(|| <usize as core::str::FromStr>::from_str(&digits).ok())
                .flatten()
        })
    }

    fn table_numbers(document: &str, label: &str) -> Vec<usize> {
        table_cell(document, label)
            .split(|character: char| !character.is_ascii_digit())
            .filter(|digits| !digits.is_empty())
            .filter_map(|digits| <usize as core::str::FromStr>::from_str(digits).ok())
            .collect()
    }

    fn table_cell<'a>(document: &'a str, label: &str) -> &'a str {
        document
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("| {label} |")))
            .unwrap_or_else(|| panic!("no `{label}` row in the closing table"))
            .split('|')
            .nth(2)
            .expect("a table value cell")
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
    fn every_live_register_row_has_a_hostile_surface_disposition() {
        const LIVE: [&str; 4] = ["REQUIRED", "READY", "VERIFY", "DECIDE"];

        let root = root();
        let register = std::fs::read_to_string(root.join("docs/016-pre-wizzy-foundation.md"))
            .expect("016-pre-wizzy-foundation.md");
        let audit =
            std::fs::read_to_string(root.join("docs/017-pre-wizzy-hostile-surface-audit.md"))
                .expect("017-pre-wizzy-hostile-surface-audit.md");
        let (_, body) = register
            .split_once("<!-- PRE-WIZZY-REGISTER:START -->")
            .expect("pre-Wizzy register start marker");
        let (body, _) = body
            .split_once("<!-- PRE-WIZZY-REGISTER:END -->")
            .expect("pre-Wizzy register end marker");
        let audit_ids = audit
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
            .filter(|token| token.starts_with("PW-"))
            .collect::<BTreeSet<_>>();
        let mut states = BTreeMap::new();
        let mut missing = Vec::new();

        for line in body.lines().filter(|line| line.starts_with("| PW-")) {
            let cells = line
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>();
            if cells.len() == 6 {
                states.insert(cells[0], cells[1]);
                if LIVE.contains(&cells[1]) && !audit_ids.contains(cells[0]) {
                    missing.push(cells[0]);
                }
            }
        }

        assert!(
            missing.is_empty(),
            "the hostile-surface audit omits live register rows: {}",
            missing.join(", ")
        );

        let mut stale = Vec::new();
        let mut misplaced = Vec::new();
        for line in audit.lines().filter(|line| line.starts_with('|')) {
            let cells = line
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>();
            // Only the proof column classifies, and only through one segment.
            if cells
                .iter()
                .enumerate()
                .any(|(index, cell)| index != 4 && cell.contains("`OPEN`"))
                || cells.get(4).is_some_and(|proof| {
                    let labels = proof.matches("`OPEN`").count();
                    labels > 1 || labels != proof.matches("`OPEN`:").count()
                })
            {
                misplaced.push(line);
            }
            let Some(proof) = cells.get(4) else {
                continue;
            };
            let Some((_, open)) = proof.split_once("`OPEN`:") else {
                continue;
            };
            let open = open.split(';').next().unwrap_or(open);
            let open_ids = open
                .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
                .filter(|token| token.starts_with("PW-"))
                .collect::<Vec<_>>();
            assert!(
                !open_ids.is_empty(),
                "the hostile-surface audit has an `OPEN` label without a register row: {line}"
            );
            for id in open_ids {
                if states.get(id).is_none_or(|state| !LIVE.contains(state)) {
                    stale.push(format!("{id} ({})", states.get(id).unwrap_or(&"unknown")));
                }
            }
        }

        assert!(
            stale.is_empty(),
            "the hostile-surface audit labels non-live register rows `OPEN`: {}",
            stale.join(", ")
        );
        assert!(
            misplaced.is_empty(),
            "an `OPEN` label must be one exact `OPEN`: segment in the proof column: {}",
            misplaced.join("\n")
        );

        let (_, cross_cutting) = audit
            .split_once("## Cross-cutting closure obligations")
            .expect("cross-cutting closure obligations heading");
        let (cross_cutting, _) = cross_cutting
            .split_once("## Stopped surfaces and audit limits")
            .expect("stopped surfaces heading");
        let mut stale_cross_cutting = Vec::new();
        for line in cross_cutting.lines().filter(|line| line.starts_with('|')) {
            let first = line
                .trim_matches('|')
                .split('|')
                .next()
                .map(str::trim)
                .unwrap_or_default();
            if first == "Register row" || first.chars().all(|character| character == '-') {
                continue;
            }
            // A row the check cannot read would otherwise escape it entirely.
            let Some(id) = first
                .strip_prefix('`')
                .and_then(|cell| cell.strip_suffix('`'))
                .filter(|cell| cell.starts_with("PW-"))
            else {
                stale_cross_cutting.push(format!("unreadable row `{first}`"));
                continue;
            };
            if states.get(id).is_none_or(|state| !LIVE.contains(state)) {
                stale_cross_cutting
                    .push(format!("{id} ({})", states.get(id).unwrap_or(&"unknown")));
            }
        }

        assert!(
            stale_cross_cutting.is_empty(),
            "the hostile-surface audit lists non-live cross-cutting obligations: {}",
            stale_cross_cutting.join(", ")
        );
    }

    fn historical_owner_disposition(
        evidence: &str,
        states: &BTreeMap<&str, &str>,
    ) -> Result<&'static str> {
        let owners = evidence
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
            .filter(|token| token.starts_with("PW-") || token.starts_with("WZ-"))
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(!owners.is_empty(), "historical handoff has no finite owner");
        let mut live = false;
        for owner in owners {
            let state = states
                .get(owner)
                .with_context(|| format!("historical handoff cites unknown owner {owner}"))?;
            match *state {
                "REQUIRED" | "READY" | "VERIFY" | "DECIDE" => live = true,
                "DONE" | "SUPERSEDED" => {}
                _ => anyhow::bail!("historical handoff owner {owner} has invalid state {state}"),
            }
        }
        Ok(if live { "READY" } else { "CLOSED" })
    }

    #[test]
    fn historical_handoffs_close_only_after_every_owner_finishes() {
        for active in ["REQUIRED", "READY", "VERIFY", "DECIDE"] {
            for finished in ["DONE", "SUPERSEDED"] {
                for states in [
                    [("PW-128", active), ("PW-201", finished)],
                    [("PW-128", finished), ("PW-201", active)],
                ] {
                    let states = states.into_iter().collect();
                    assert_eq!(
                        historical_owner_disposition("PW-128 and `PW-201`.", &states).unwrap(),
                        "READY",
                        "live owner {active} was hidden"
                    );
                }
            }
        }
        for first in ["DONE", "SUPERSEDED"] {
            for second in ["DONE", "SUPERSEDED"] {
                let states = [("PW-128", first), ("PW-201", second)]
                    .into_iter()
                    .collect();
                assert_eq!(
                    historical_owner_disposition("PW-128 and PW-201", &states).unwrap(),
                    "CLOSED",
                    "completed owners stayed live"
                );
            }
        }
        let states = [("PW-128", "DONE")].into_iter().collect();
        assert!(historical_owner_disposition("no owner", &states).is_err());
        assert!(historical_owner_disposition("PW-128 and PW-201", &states).is_err());
        for invalid in ["TRIGGER", "WIZZY", "", "UNKNOWN"] {
            let states = [("PW-128", "READY"), ("PW-201", invalid)]
                .into_iter()
                .collect();
            assert!(
                historical_owner_disposition("PW-128 and PW-201", &states).is_err(),
                "owner state {invalid:?} was admitted"
            );
        }
    }

    #[test]
    fn every_historical_handoff_has_one_current_disposition() {
        const TRIGGERS: [usize; 4] = [121, 129, 143, 154];
        const REVIEW_LIMITS: [usize; 5] = [124, 128, 134, 140, 150];
        const READY: [usize; 2] = [142, 148];
        const CLOSED_OR_SUPERSEDED: [usize; 30] = [
            122, 123, 124, 125, 126, 127, 128, 130, 131, 132, 133, 134, 135, 137, 138, 139, 140,
            141, 136, 144, 145, 146, 147, 149, 150, 151, 152, 153, 155, 156,
        ];

        let register = std::fs::read_to_string(root().join("docs/016-pre-wizzy-foundation.md"))
            .expect("016-pre-wizzy-foundation.md");
        let (_, reconciliation) = register
            .split_once("### Historical D-121 through D-156 handoff reconciliation")
            .expect("the historical handoff reconciliation");
        let (reconciliation, _) = reconciliation
            .split_once("## What is left")
            .expect("the end of the historical handoff reconciliation");
        let mut dispositions = BTreeMap::<usize, (&str, &str)>::new();

        for line in reconciliation.lines().filter(|line| line.starts_with('|')) {
            let cells = line
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>();
            if cells.len() != 3 {
                continue;
            }
            for token in cells[0]
                .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
                .filter_map(|token| token.strip_prefix("D-"))
                .filter_map(|number| <usize as core::str::FromStr>::from_str(number).ok())
                .filter(|number| (121..=156).contains(number))
            {
                assert!(
                    dispositions.insert(token, (cells[1], cells[2])).is_none(),
                    "D-{token} has more than one current disposition"
                );
            }
        }

        let expected = (121..=156).collect::<BTreeSet<_>>();
        let actual = dispositions.keys().copied().collect::<BTreeSet<_>>();
        assert_eq!(
            actual, expected,
            "the historical handoff reconciliation is incomplete"
        );

        let register_states = register
            .lines()
            .filter_map(|line| {
                let mut cells = line.trim_start().strip_prefix("| ")?.split('|');
                let id = core::iter::Iterator::next(&mut cells)?.trim();
                let state = core::iter::Iterator::next(&mut cells)?.trim();
                (id.starts_with("PW-") || id.starts_with("WZ-")).then_some((id, state))
            })
            .collect::<BTreeMap<_, _>>();
        for (decision, (_, evidence)) in &dispositions {
            for reference in evidence
                .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
                .filter(|token| token.starts_with("PW-") || token.starts_with("WZ-"))
            {
                assert!(
                    register_states.contains_key(reference),
                    "D-{decision} cites unknown register row {reference}"
                );
            }
        }

        for decision in TRIGGERS {
            let (state, trigger) = dispositions[&decision];
            assert_eq!(state, "TRIGGER", "D-{decision} is stopped work");
            assert!(
                trigger.contains("PW-70")
                    || trigger.contains("Phase 12")
                    || trigger.contains("post-Wizzy"),
                "D-{decision} has no exact current trigger"
            );
        }
        for decision in REVIEW_LIMITS {
            assert!(
                dispositions[&decision].0.contains("REVIEW LIMIT"),
                "D-{decision} hides its non-automatable review boundary"
            );
        }
        for decision in READY {
            let (state, evidence) = dispositions[&decision];
            let expected = historical_owner_disposition(evidence, &register_states)
                .unwrap_or_else(|error| panic!("D-{decision}: {error}"));
            assert_eq!(
                state, expected,
                "D-{decision} disagrees with its finite owners"
            );
        }
        for decision in CLOSED_OR_SUPERSEDED {
            let state = dispositions[&decision].0;
            assert!(
                state.contains("CLOSED") || state.contains("SUPERSEDED"),
                "D-{decision} does not name its later closure or supersession"
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
