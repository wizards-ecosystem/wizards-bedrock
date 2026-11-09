# D-22 Lint, test, and CI policy

Status: Accepted, 2026-08-12.

**Decision.** A `[workspace.lints]` table all kernel crates opt into:
`unsafe_op_in_unsafe_fn`, `unused_must_use` denied; `missing_docs`,
`unreachable_pub` at warn; and from clippy's allow-by-default `restriction`
group, `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`, and
`missing_safety_doc` denied. `clippy::missing_docs_in_private_items` is
deliberately **not** enabled. `cargo xtask ci` is the single gate.
*(A GitHub Actions workflow ran the same steps on Linux for a day; removed by
D-26.)*

**Rationale.** `undocumented_unsafe_blocks` is what mechanically forces a
`// SAFETY:` on every unsafe operation, and its companion keeps each comment
covering exactly one operation. Together they turn D-13's discipline from an
aspiration into a build failure. The excluded lint is the one that would
demand a doc comment on every private helper, which is noise; the policy is
"prose where it carries a safety obligation, nowhere else"
(`005-code-conventions.md`).

**Amended 2026-08-13.** The gate was linting only the library targets: neither
`check` nor `clippy` passed `--all-targets`, so no test code in the workspace
had ever been linted, and a `useless_vec` in `sha256.rs`'s tests had been sitting
there unreported. `--all-targets` now goes to the host half — and only the host
half, because the kernel crates have no test or bench targets and cannot have
any (D-28). The argument construction moved into a pure `half_args` function
with tests, so the distinction is asserted rather than remembered.

**Amended 2026-08-22, after an adversarial audit.** Three holes, all of the
same species as the one above — the gate covering less than it was believed
to.

1. *The configuration that ships was never linted.* Every kernel pass carried
   `--features wizard-kernel/selftest`, so `cargo xtask ci` linted exactly one
   crate graph and the release build in its own last step linted none. Two
   `dead_code` constants used only by `selftest` had been riding release
   images out, printed as warnings by a step nothing gated. The pass list is
   now an explicit `PASSES` table — host, kernel, kernel + self-test — with
   tests asserting the kernel appears twice and that the shipped
   configuration comes first.
2. *`kernel/` never opted in at all.* It had no `[lints]` table, so every lint
   this ADR describes was unenforced in the crate that most needed them — the
   `unreachable_pub` that `005` says enforces `pub(crate)` in a binary was
   enforcing nothing. It opts in now. The code turned out to already conform,
   which is the good outcome and not the point: it was conforming by habit.
3. *Doc links rot unwatched.* `missing_docs` catches an absent doc; nothing
   caught one pointing at a renamed or now-private item, because those lints
   fire only under rustdoc and no gate step ran it. `[workspace.lints.rustdoc]`
   denies `broken_intra_doc_links`, `private_intra_doc_links` and
   `redundant_explicit_links`, and `cargo xtask doc` makes them bite. Two dead
   links were found on the first run. `xtask` takes this table on its own,
   still opting out of the rest.
4. *The package list drifted, because there are two of them.* `HOST_PACKAGES`
   in `xtask/src/paths.rs` and `default-members` in the root manifest say the
   same thing twice, and adding `libs/port-space` to one and not the other
   made the gate stop covering it while a bare `cargo test` still did — found
   while fixing hole 1, by the same kind of looking. `paths::default_members`
   now reads the real manifest in a unit test and requires the two to agree.
   A list duplicated without a check between the copies is a defect with a
   waiting period.

**Consequences.** Cargo forbids mixing `[lints] workspace = true` with local
lint tables in the same manifest, so per-crate escalation goes in source
attributes — `#![deny(unsafe_code)]` sits at the top of `kernel/src/main.rs`,
where a reader meets it in the first five lines. `xtask` opts out of the
workspace lints bar the rustdoc ones: `missing_docs` on build tooling is
noise, a link that no longer resolves is a defect anywhere. Automated tests
exist for host-side logic only; the reasoning and the path forward are in
`005-code-conventions.md`.

The standing lesson, now twice learned: **a gate is only as wide as its
configuration matrix, and the matrix is the thing to audit.** Every widening
so far has been found by looking, never by the gate itself.
