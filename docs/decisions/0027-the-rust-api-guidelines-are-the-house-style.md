# D-27 The Rust API Guidelines are the house style

Status: Accepted, 2026-08-12.

**Decision.** Adopt the [Rust API
Guidelines](https://rust-lang.github.io/api-guidelines/) as the default
standard, and the [Rust for Linux coding
guidelines](https://docs.kernel.org/rust/coding-guidelines.html) where they
are more specific about kernel work: chiefly the `// SAFETY:` / `# Safety`
distinction and the rule against repeating a namespace inside an identifier.
`005` cites the checklist items it turns into rules.

**Rationale.** Matching conventions a Rust programmer already knows makes
the code read correctly to a newcomer and gives disagreements an external
referee instead of taste.

**Consequences.** `missing_debug_implementations` is in the lint table
(C-DEBUG); every error type implements `Display` and `core::error::Error`
(C-GOOD-ERR) and error enums are `#[non_exhaustive]`; named types replace
`bool` parameters (C-CUSTOM-TYPE). Two guidelines are **not** adopted, with
reasons in `005`: C-EXAMPLE, because `no_std` cross-compiled crates cannot
run doctests, and Rust for Linux's one-import-per-line layout, which
contradicts the `rustfmt` defaults.
