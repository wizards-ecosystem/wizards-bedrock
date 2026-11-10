# D-27 The Rust API Guidelines are the house style

Status: Accepted, 2026-08-12.

**Context.** `005-code-conventions.md` came from the project's own
experience. Before subsystems were built on it, it was checked against the
established sources instead of becoming a local dialect.

**Decision.** Adopt the [Rust API
Guidelines](https://rust-lang.github.io/api-guidelines/) as the default
standard, and the [Rust for Linux coding
guidelines](https://docs.kernel.org/rust/coding-guidelines.html) where they
are more specific about kernel work: chiefly the `// SAFETY:` / `# Safety`
distinction and the rule against repeating a namespace inside an identifier.
`005` now cites the checklist items it turns into rules.

**Rationale.** A Rust programmer already knows these conventions. Matching
them makes the code read correctly to a newcomer and gives disagreements an
external referee instead of taste.

**Consequences.** Four guideline violations were found and fixed in the
existing code:

- `missing_debug_implementations` added to the lint table and `Debug`
  implemented across `wost` (C-DEBUG);
- `Display` and `core::error::Error` on every error type (C-GOOD-ERR);
- `bool` parameters replaced with named types: `FmtMode`, `CleanScope`, and
  a `Palette` that stops two same-typed `Rgb` arguments being passed the
  wrong way round (C-CUSTOM-TYPE);
- `#[non_exhaustive]` on the error enums (C-STRUCT-PRIVATE's
  future-proofing rationale).

Two guidelines are **not** adopted, with reasons recorded in `005`.
C-EXAMPLE (a rustdoc example on every item) is out because `no_std`
cross-compiled crates cannot run doctests. Rust for Linux's
one-import-per-line layout is out because it contradicts the `rustfmt`
defaults this project formats with.
