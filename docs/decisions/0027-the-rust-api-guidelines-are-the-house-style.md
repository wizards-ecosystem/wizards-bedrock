# D-27 The Rust API Guidelines are the house style

Status: Accepted (2026-08-12)

**Context.** `005-code-conventions.md` was written from the project's own
experience. Before building out subsystems on top of it, it was worth checking
against the established sources rather than inventing a local dialect.

**Decision.** Adopt the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
as the default standard, and the [Rust for Linux coding
guidelines](https://docs.kernel.org/rust/coding-guidelines.html) where they are
more specific about kernel work — chiefly the `// SAFETY:` / `# Safety`
distinction and the rule against repeating a namespace inside an identifier.
`005` now cites the specific checklist items it turns into rules.

**Rationale.** These are the conventions a Rust programmer already has in their
head. Matching them means the code reads correctly to someone who has never
seen this project, and disagreements have an external referee rather than being
a matter of taste.

**Consequences.** Four guideline violations were found and fixed in the existing
code: `missing_debug_implementations` added to the lint table and `Debug`
implemented across `wost` (C-DEBUG); `Display` and `core::error::Error` on every
error type (C-GOOD-ERR); `bool` parameters replaced with named types —
`FmtMode`, `CleanScope`, and a `Palette` that stops two same-typed `Rgb`
arguments being passed the wrong way round (C-CUSTOM-TYPE); and `#[non_exhaustive]`
on the error enums (C-STRUCT-PRIVATE's future-proofing rationale).

Two guidelines are explicitly **not** adopted, with reasons recorded in `005`:
C-EXAMPLE (a rustdoc example on every item) because `no_std` cross-compiled
crates cannot run doctests, and Rust for Linux's one-import-per-line layout
because it contradicts the `rustfmt` defaults this project formats with.
