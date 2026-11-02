# Stream 06 — integration

Branch: `integration/bedrock-v0.3-prep`
Base: `origin/main` at `94bfcce` (Close the rows v0.2.0 proved).
Do not merge this branch into `main`. The integration pull request stays open for review.

## Done

- Review comments posted on the five drafts. Comment, not approval.
  - #6 lint foundation
  - #7 libs
  - #5 kernel
  - #4 boot tooling
  - #3 roadmap
- Merged #6 (`cursor/beautiful-rust-lints-b7b1`) with a merge commit.
  Safety denies stay deny. Pedantic lints are warn. `cargo xtask clippy`
  still passes `-D warnings`.

## In progress

Merges still to land, in order: #7, #5, #4, #3. Then the cross-cutting
signature changes, the D-159 amendment, and `cargo xtask ci`.

## Next steps

1. Merge `cursor/libs-soundness-and-lints-8fd3` (#7).
2. Merge `cursor/kernel-review-e5bf` (#5).
3. Merge `cursor/boot-tooling-dx-091e` (#4).
4. Merge `cursor/roadmap-v030-docs-bb4c` (#3).
5. Change the kernel-facing signatures listed in
   `handoff/streams/04-boot-tooling.md`, and `vtd::second_level::pte`'s
   bool parameters from `handoff/streams/03-libs.md`.
6. Clear the remaining pedantic `wost` warnings that stream 4 listed,
   where a local edit is honest.
7. Amend D-159 to record the warn-level pedantic rollout. Keep
   `cargo xtask clippy` denying warnings. Narrowing casts stay
   per-site `try_from` or `#[expect]` with a reason.
8. Leave mutation-pinned text alone: `require_unique`, the ACPI
   duplicate-table branch, and mutation 1819.
9. Run `cargo xtask ci` and the host, kernel, clippy, and QEMU steps
   that the repository defines and that run on Linux. The Windows-only
   codegen-path test (mutation 1789) is expected to fail on Linux.
10. Open one pull request to `main`. Do not merge it.

## Decisions

- Review comments describe incorrect decode and wrong port claims in
  neutral wording. No vulnerability write-up.
- Isaac's lead, recorded here so a resume does not relitigate it:
  `cargo xtask clippy` keeps `-D warnings`. Truncation is answered per
  site. D-159 is amended rather than replaced. Isaac may override.
