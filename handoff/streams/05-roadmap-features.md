# Stream 5 — roadmap and features

Owner directories: `docs/` except `docs/style/`, `README.md`, `fixtures/`,
`models/`. Also this handoff file and `experiments/`, which did not exist
before this stream. Other streams own root config and CI, `kernel/`, `libs/`,
and `boot/` `xtask/` `tools/` `wost/`.

Branch: `cursor/roadmap-v030-docs-bb4c`.
Draft pull request: https://github.com/wizards-ecosystem/wizards-bedrock/pull/3

## Done

- Read `AGENTS.md`, `CONTRIBUTING.md`, `README.md`, `docs/`, `handoff/`, and
  GitHub issues 1 and 2.
- Confirmed the workspace version is 0.2.0 and that D-449 closed the rows
  v0.2.0 proved. Live register rows are PW-006 (`REQUIRED`) and PW-010
  (`READY`). `TRIGGER` and `WIZZY` rows stay stopped.
- Wrote `docs/roadmap/NEXT.md`, the v0.3.0 assignment index, with sizes and
  directories. Issue 1 is recorded as met by v0.2.0. Issue 2 has a Windows
  procedure and stays open because this environment cannot run it.
- Corrected current-facing status in `README.md`, `docs/014-storage.md`,
  `docs/017-pre-wizzy-hostile-surface-audit.md`, and `docs/releases/v0.1.0.md`.
  Dated decision records were left dated.
- Added README "Start here" with the four public commands, channel `1.97.1`,
  and the statement that a successful build is not a boot.
- Added `experiments/register-index`, a separate Cargo workspace that prints
  register rows. `cargo test --manifest-path experiments/register-index/Cargo.toml --offline`
  passed on Linux with rustc 1.97.1: 4 tests, including the check that the
  current register's live rows are exactly PW-006 and PW-010. `--live` printed
  those two rows from 225 parsed rows.

## In progress

- Nothing. The draft pull request still needs its description brought up to
  this commit.

## Next steps

- Update the pull request body. Leave it a draft. Do not merge.
- Do not run `cargo xtask ci`, QEMU, or the mutation sweep. The integration
  pass after all streams finish owns that.
- Issue 2 still needs a clean Windows GNU report from someone who can run one.
  If that report disagrees with the README, edit `README.md` only unless the
  failure is a toolchain or `xtask` defect.
- PW-006 and PW-010 are the next kernel work. Follow the directory split in
  `docs/roadmap/NEXT.md`. Do not start `TRIGGER` rows.

## Decisions

- Issue 1 needs no source change. v0.2.0 already published the separate
  qualified release the issue describes. Closing the GitHub issue is a
  maintainer action; this stream does not write to GitHub issues.
- Issue 2 stays open. A Linux reading of the quickstart is not the clean
  Windows report the issue requires.
- `docs/roadmap/NEXT.md` is an assignment index. It does not add register
  rows and does not authorize `TRIGGER` work or a Rust userland.
- The register index lives under `experiments/` so it does not join the kernel
  workspace, `libs/`, or `cargo xtask ci`. The Markdown register stays
  authoritative.
- No security-vulnerability detail is added to the roadmap, the README, or
  the pull request. The audit stays identified as PW-006 and PW-010.
