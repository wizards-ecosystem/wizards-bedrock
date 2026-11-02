# Stream 01 — lint foundation

Owner: workspace root config only (`Cargo.toml` lints, `clippy.toml`,
`rustfmt.toml`, `rust-toolchain.toml`, `.cargo/`, `.github/`, `docs/style/`).
No edits under `kernel/`, `libs/`, `boot/`, `xtask/`, `tools/`, or `wost/`.

Branch: `cursor/beautiful-rust-lints-b7b1`
Draft PR: https://github.com/wizards-ecosystem/wizards-bedrock/pull/6

## Done

- Workspace `[lints]` inherits BeautifulRust's tested table. Safety denies
  stay deny. New pedantic lints are warn. `clippy.toml`, `rustfmt.toml`, and
  `.cargo/config.toml` match that.
- `docs/style/BEAUTIFUL_RUST.md` is the rule sheet, including `// SAFETY:` /
  `# Safety` and the kernel allows.
- Source-preview workflow caches the Cargo registry and `target/`, and runs
  `cargo clippy --all-targets --locked` plus `cargo kclippy --locked` without
  `-D warnings`. No new boot or test job. Timeout stays 20 minutes.
  Self-test and `refuse-pci` are not linted in that job.
- `rust-toolchain.toml` unchanged (already pins 1.97.1 and installs rustfmt
  and clippy).
- Warning census below, from rustc 1.97.1 on Linux. Every pass exited 0, so
  the denies still hold. `cargo metadata --no-deps` parsed the manifest.
  Full `cargo xtask ci`, QEMU, and the host test suite were not run.

## In progress

Nothing. Waiting on Isaac for the open questions, and on other streams for
the per-crate warns.

## Next steps

1. Do not merge the draft PR.
2. Other streams fix warns in the crates they own, using the style guide.
   A register-width truncation is an `expect` with a reason, not a rewrite.
3. Re-run the census after those edits. `cargo xtask clippy` stays red until
   the warns are gone or the gate stops passing `-D warnings`.

## Decisions

- New lints are warn, not deny, so `cargo check` / `cargo build` / plain
  `cargo clippy` stay green. `cargo xtask clippy` still appends `-D warnings`.
- Relaxed: `expect_used`, `as_conversions`, `cast_sign_loss`,
  `cast_possible_wrap`, `enum_variant_names`, plus BeautifulRust's allows
  (`module_name_repetitions`, `must_use_candidate`, `module_inception`,
  `enum_glob_use`). `indexing_slicing`, `missing_docs_in_private_items`, and
  `clippy::nursery` stay off.
- `cast_possible_truncation` stays warn. It is the majority of the census.
  Whether to allow it is an open question, not a silent flip.
- `xtask` does not inherit the table (D-22).
- No new decision-log number. D-159 still says these groups were not adopted.
  This stream did not edit `docs/004-decision-log.md`.

## Warning counts

Unique `(file, line, lint)` findings. `cargo clippy --all-targets` emits some
host findings twice (library target and tests); the unique number is the
number of sites. Kernel numbers are the `x86_64-unknown-none` graph. Raw
diagnostic totals are in parentheses when they differ.

Host command: `cargo clippy --all-targets --locked` (exit 0, 591 raw warns).
Shipped kernel: `cargo kclippy --locked` (exit 0, 503 raw, libs included).
Self-test: `cargo kclippy --locked --features wizard-kernel/selftest` (exit 0).
`refuse-pci` added no findings beyond the shipped kernel.

### Libs and xtask (host `--all-targets`)

The second number is the same crate on the shipped kernel target (no tests,
`no_std`). Fix the host sites, then recheck `cargo kclippy`, because the
graphs are not the same.

| Crate | Host unique | Kernel-target unique | What dominates on the host |
|---|---:|---:|---|
| acpi-tables | 19 | 6 | truncation |
| eevdf | 5 | 4 | truncation |
| elf64 | 6 | 2 | truncation, lossless cast |
| fat32 | 19 | 6 | truncation, exhaustive enums |
| frame-space | 7 | 5 | truncation, missing errors doc |
| handle-space | 5 | 5 | `return_self_not_must_use` |
| memory-map | 4 | 1 | exhaustive enums |
| mmio-range | 3 | 3 | `manual_let_else` |
| nvme | 15 | 14 | truncation, missing errors doc |
| panic-record | 0 | 0 | |
| path-walk | 1 | 0 | truncation |
| pci-config | 83 | 65 | truncation (about half), doc markdown, exhaustive enums |
| pixel | 13 | 8 | truncation |
| port-space | 0 | 0 | |
| ps2-keys | 14 | 8 | doc markdown, exhaustive enums |
| ring-proto | 9 | 1 | missing assert message |
| slab-layout | 1 | 1 | exhaustive enums |
| stack-layout | 5 | 1 | truncation |
| timebase | 8 | 5 | truncation |
| trap-cause | 5 | 2 | `wrong_self_convention`, bit mask, lossless cast |
| vtd | 9 | 7 | doc markdown |
| wizzy-handoff | 0 | 0 | |
| x86-apic | 17 | 17 | exhaustive enums, missing errors doc |
| x86-cpuid | 16 | 13 | truncation, `return_self_not_must_use`, lossless cast |
| x86-paging | 19 | 17 | exhaustive enums |
| x86-pit | 11 | 4 | exhaustive enums, doc markdown |
| x86-tables | 26 | 5 | lossless cast, truncation |
| xtask | 3 | n/a | pre-existing `dead_code` only; this table does not apply |

Host unique total, libs + xtask: 323 (320 of them in `libs/`).

### Kernel crates (shipped, then self-test)

| Crate | Shipped unique | Self-test unique | Extra sites only in self-test |
|---|---:|---:|---:|
| kernel | 86 | 204 | 118 |
| wost | 211 | 234 | 24 |

Shipped `kernel` is mostly truncation, doc markdown, `manual_assert`,
`single_match_else`, `too_many_lines`. Shipped `wost` is mostly truncation,
missing `# Errors` / `# Panics`, `panic_in_result_fn`, `needless_pass_by_value`,
and `cast_ptr_alignment`. `refuse-pci` matched the shipped counts.

`xtask` `dead_code` (Linux): `FILE_READ_DATA_ACCESS` in
`xtask/src/security_deep.rs`, `parked_family` and `released` in
`xtask/src/bounded_process.rs`. Not introduced by this table. This stream
does not edit `xtask`.

## How to resume

Levels: root `Cargo.toml` `[workspace.lints]`. Thresholds: `clippy.toml`.
Rules: `docs/style/BEAUTIFUL_RUST.md`. Census JSON is not in the repo; rerun
the four commands above with `--message-format=json` if the counts need to
move. Do not edit crate sources from this stream. Do not run `cargo xtask ci`,
QEMU, or the full test suite.
