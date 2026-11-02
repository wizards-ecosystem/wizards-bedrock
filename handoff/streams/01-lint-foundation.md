# Stream 01 — lint foundation

Owner: workspace root config only (`Cargo.toml` lints, `clippy.toml`,
`rustfmt.toml`, `rust-toolchain.toml`, `.cargo/`, `.github/`, `docs/style/`).
No edits under `kernel/`, `libs/`, `boot/`, `xtask/`, `tools/`, or `wost/`.

Branch: `cursor/beautiful-rust-lints-b7b1`

## Done

- Read AGENTS.md, CONTRIBUTING.md, D-22 / D-159 / D-27, and BeautifulRust's
  tested `lints/lints.toml`, `lints/clippy.toml`, and the three skills
  (`rust-best-practices`, `rust-review`, `rust-port`).
- Workspace `[lints]` now inherits BeautifulRust's tested table. Existing
  safety denies stay deny. New pedantic lints are warn.
- `clippy.toml` sets the bool-parameter threshold, test/const panic allows,
  doc acronyms, and `avoid-breaking-exported-api = false`.
- `docs/style/BEAUTIFUL_RUST.md` is the contributor and agent summary,
  including `// SAFETY:` / `# Safety` and the kernel allows.
- `rustfmt.toml` records why Google's extra knobs stay unset on rustfmt 1.9.
- `.cargo/config.toml` notes that plain clippy exits 0 on these warns and
  `cargo xtask clippy` does not.

## In progress

- Fast clippy, to confirm the lint names exist on 1.97.1 and to count warns
  per crate.
- Source-preview CI: Cargo cache and a clippy step that does not pass
  `-D warnings`.

## Next steps

1. Finish the per-crate warning census (host `--all-targets`, shipped kernel,
   self-test, `refuse-pci`) and paste the counts here and in the PR.
2. Add the CI cache and clippy step. Keep the 20-minute job budget. Do not
   add boot tests or `-D warnings`.
3. Leave the PR draft. Do not merge.

## Decisions

- New lints are warn, not deny, so `cargo check` / `cargo build` stay green
  on other streams. `cargo xtask clippy` still denies all warnings; that
  interaction is an open question for Isaac, because this stream does not
  edit `xtask`.
- Relaxed, with reasons in the style guide: `expect_used`, `as_conversions`,
  `cast_sign_loss`, `cast_possible_wrap`, `enum_variant_names`, plus
  BeautifulRust's own allows (`module_name_repetitions`, `must_use_candidate`,
  `module_inception`, `enum_glob_use`). `cast_possible_truncation` stays warn.
  `indexing_slicing`, `missing_docs_in_private_items`, and `clippy::nursery`
  stay off (D-159, comment noise, toolchain churn).
- Safety denies are unchanged and stricter than BeautifulRust's warn.
- `xtask` still does not inherit workspace lints (D-22).
- No new decision-log number. D-159 says these groups were not adopted.
  Amending that entry is Isaac's call; this stream does not edit
  `docs/004-decision-log.md`.
- `rust-toolchain.toml` needs no change: the pin is already exact (D-24) and
  already installs rustfmt and clippy.

## Warning counts

Not run yet.

## How to resume

Lint levels: root `Cargo.toml` `[workspace.lints]`. Thresholds: `clippy.toml`.
Rules: `docs/style/BEAUTIFUL_RUST.md`. Do not edit crate sources from this
stream. Do not run `cargo xtask ci`, QEMU, or the full test suite.
