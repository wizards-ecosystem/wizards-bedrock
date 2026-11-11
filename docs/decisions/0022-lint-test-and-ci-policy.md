# D-22 Lint, test, and CI policy

Status: Accepted, 2026-08-12.

**Decision.** All kernel crates opt into one `[workspace.lints]` table:
`unsafe_op_in_unsafe_fn` and `unused_must_use` denied; `missing_docs` and
`unreachable_pub` at warn. From clippy's allow-by-default `restriction`
group, `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block` and
`missing_safety_doc` are denied. `clippy::missing_docs_in_private_items` is
**not** enabled. `cargo xtask ci` is the single gate (D-26).

**Rationale.** `undocumented_unsafe_blocks` forces a `// SAFETY:` on every
unsafe operation, and its companion keeps each comment covering exactly one
operation, so D-13's discipline is a build failure. The policy is "prose
where it carries a safety obligation, nowhere else"
(`005-code-conventions.md`); a doc comment on every private helper is noise.

**Amended 2026-08-13.** `--all-targets` goes to the host half only; the
kernel crates have no test or bench targets and cannot have any (D-28). The
argument construction is a pure, tested `half_args` function, so the
distinction is asserted, not remembered.

**Amended 2026-08-22.** The gate must cover what it is believed to cover:

1. *The shipped configuration is linted.* The pass list is an explicit
   `PASSES` table (host, kernel, kernel with self-test); tests assert the
   kernel appears twice and the shipped configuration comes first.
2. *`kernel/` opts in* to the workspace lints, so `unreachable_pub` enforces
   `pub(crate)` in the binary as `005` says.
3. *Doc links are checked.* `missing_docs` catches an absent doc. Nothing
   catches one pointing at a renamed or now-private item except rustdoc, so
   `[workspace.lints.rustdoc]` denies `broken_intra_doc_links`,
   `private_intra_doc_links` and `redundant_explicit_links`, and
   `cargo xtask doc` enforces them. `xtask` takes this table alone and still
   opts out of the rest.
4. *The two package lists agree.* `HOST_PACKAGES` in `xtask/src/paths.rs`
   and `default-members` in the root manifest say the same thing;
   `paths::default_members` reads the manifest in a unit test and requires
   the two to agree. A list duplicated without a check between the copies is
   a defect with a waiting period.

**Consequences.** Cargo forbids mixing `[lints] workspace = true` with local
lint tables in one manifest, so per-crate escalation goes in source
attributes; `#![deny(unsafe_code)]` sits in the first five lines of
`kernel/src/main.rs`. `xtask` opts out of the workspace lints except the
rustdoc ones: `missing_docs` on build tooling is noise, and a link that no
longer resolves is a defect anywhere.

The standing lesson: **a gate is only as wide as its configuration matrix,
and the matrix is the thing to audit.**
