# D-22 Lint, test, and CI policy

Status: Accepted, 2026-08-12.

**Decision.** All kernel crates opt into one `[workspace.lints]` table:
`unsafe_op_in_unsafe_fn` and `unused_must_use` denied; `missing_docs` and
`unreachable_pub` at warn. From clippy's allow-by-default `restriction`
group, `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block` and
`missing_safety_doc` are denied. `clippy::missing_docs_in_private_items` is
**not** enabled. `cargo xtask ci` is the single gate. *(A GitHub Actions
workflow ran the same steps on Linux for a day; removed by D-26.)*

**Rationale.** `undocumented_unsafe_blocks` forces a `// SAFETY:` on every
unsafe operation, and its companion keeps each comment covering exactly one
operation. Together they make D-13's discipline a build failure. The
excluded lint would demand a doc comment on every private helper, which is
noise. The policy is "prose where it carries a safety obligation, nowhere
else" (`005-code-conventions.md`).

**Amended 2026-08-13.** The gate linted only library targets: neither `check`
nor `clippy` passed `--all-targets`, so no workspace test code had ever been
linted. A `useless_vec` in `sha256.rs`'s tests went unreported.
`--all-targets` now goes to the host half only; the kernel crates have no test
or bench targets and cannot have any (D-28). The argument construction moved
into a pure, tested `half_args` function, so the distinction is asserted, not
remembered.

**Amended 2026-08-22, after an adversarial audit.** Three holes of the same
kind: the gate covered less than it was believed to.

1. *The configuration that ships was never linted.* Every kernel pass
   carried `--features wizard-kernel/selftest`, so `cargo xtask ci` linted
   one crate graph, and the release build in its last step linted none. Two
   `dead_code` constants used only by `selftest` shipped in release images,
   printed as warnings by an ungated step. The pass list is now an explicit
   `PASSES` table (host, kernel, kernel with self-test); tests assert the
   kernel appears twice and the shipped configuration comes first.
2. *`kernel/` never opted in.* With no `[lints]` table, every lint this ADR
   describes was unenforced in the crate that most needed them; the
   `unreachable_pub` that `005` says enforces `pub(crate)` in a binary
   enforced nothing. It opts in now. The code already conformed, but only by
   habit.
3. *Doc links rot unwatched.* `missing_docs` catches an absent doc, but not
   one pointing at a renamed or now-private item: those lints fire only
   under rustdoc, which no gate step ran. `[workspace.lints.rustdoc]` denies
   `broken_intra_doc_links`, `private_intra_doc_links` and
   `redundant_explicit_links`, and `cargo xtask doc` enforces them. The
   first run found two dead links. `xtask` takes this table alone and still
   opts out of the rest.
4. *The package list drifted, because there are two of them.*
   `HOST_PACKAGES` in `xtask/src/paths.rs` and `default-members` in the root
   manifest say the same thing. Adding `libs/port-space` to one only made
   the gate stop covering it while a bare `cargo test` still did. It was
   found while fixing hole 1, by the same kind of looking.
   `paths::default_members` now reads the manifest in a unit test and
   requires the two to agree. A list duplicated without a check between the
   copies is a defect with a waiting period.

**Consequences.** Cargo forbids mixing `[lints] workspace = true` with local
lint tables in one manifest, so per-crate escalation goes in source
attributes. `#![deny(unsafe_code)]` sits at the top of `kernel/src/main.rs`,
in the first five lines a reader meets. `xtask` opts out of the workspace
lints except the rustdoc ones: `missing_docs` on build tooling is noise, and
a link that no longer resolves is a defect anywhere. Automated tests cover
host-side logic only; the reasoning and path forward are in
`005-code-conventions.md`.

The standing lesson, learned twice: **a gate is only as wide as its
configuration matrix, and the matrix is the thing to audit.** Every widening
so far was found by looking, never by the gate.
