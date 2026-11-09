# D-451 Decisions, documentation and code voice

Status: Accepted, 2026-11-09. Amends D-19, D-22, D-131 and D-159.

**Context.** The decision log was a single file of 450 entries. It is now one
file per decision, with `docs/004-decision-log.md` as the index. D-19 numbers
docs in creation order but says nothing of decision files. Docs 018 to 020
only point at the Realm specifications D-366 transferred, and no doc states
how Bedrock's prose is written. Code comments cite decisions, so a comment
falls behind when its decision is amended. The lint policy is spread over
D-22, D-131 and D-159, and `xtask` stays outside the workspace lints because
of `missing_docs`.

**Decision.** Decisions, documentation and code follow these rules.

1. Each decision is its own file, `docs/decisions/NNNN-slug.md`, with `NNNN`
   the zero-padded number. It opens with `# D-N Title`, a blank line and a
   `Status:` line of the form `Status: <status>, <date>. <relations>.` The
   slug is frozen once written: a later title edit does not rename the file.
   `docs/004-decision-log.md` is the index and lists every file once, in
   order. Decisions are never renumbered, as D-19 already rules for docs.
2. The documentation voice standard is a section of `005-code-conventions.md`.
   Docs 018 to 020 retire, and their numbers are not reused.
3. Code comments never cite a decision. A decision file names the code it
   governs by path and item, so the reference runs from the decision to the
   code and never back. This extends D-19's docs-before-code method.
4. Every crate, `xtask` included, opts into one workspace lint table: the lint
   table shared across the Wizards repositories, without `missing_docs`. This
   replaces D-22's `missing_docs` at warn and its `xtask` opt-out. A
   suppression is `#[expect(lint, reason = "...")]`. It stays
   `#[allow(lint, reason = "...")]` only where D-131 shows `expect` cannot
   reach: inside a macro expansion, or on a standing permission such as
   `wost`'s `unsafe_code` lift. `clippy::allow_attributes_without_reason`
   refuses a suppression without a reason. `clippy::pedantic` is at warn, and
   each pedantic lint left off is named in the table as an allow, which
   amends D-159's choice to adopt none. A lint is enabled only in the commit
   that brings its count to zero, so the gate never carries a warning.
5. The evidence reader's `parent-docs-only-v1` policy admits only in-place
   modifications of five named files, among them the index
   `docs/004-decision-log.md`. It refuses an added file and every path under
   `docs/decisions/`, so a documentation commit after a qualified source can
   edit the index but never adds or edits a decision file. Admitting decision
   files would be a `parent-docs-only-v2` policy and its own decision.

**Consequences.** The decision reader already refuses a file that does not
open with its heading, a blank line and a `Status:` line, a numbering gap and
an index that disagrees with the files. Rule 5 describes the policy as
`xtask/src/evidence.rs` enforces it. Rules 2 to 4 are the target the source
coherence pass reaches; until it lands,
comments that cite decisions, the stub docs and the current lint tables
remain. The bodies of D-19, D-22, D-131 and D-159 are not edited.

**Alternatives.** One log file is a 25,937-line document that no reader opens
whole. Renaming a file when its title changes would break every link to it.
Decision citations in comments leave a second place to update for each
amendment, and nothing checks the comment.
