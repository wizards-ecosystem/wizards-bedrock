# Stream 5 — roadmap and features

Owner directories: `docs/` except `docs/style/`, `README.md`, `fixtures/`,
`models/`. Also this handoff file. Other streams own root config and CI,
`kernel/`, `libs/`, and `boot/` `xtask/` `tools/` `wost/`.

Branch: `cursor/roadmap-v030-docs-bb4c`.

## Done

- Read `AGENTS.md`, `CONTRIBUTING.md`, `README.md`, `docs/`, `handoff/`, and
  GitHub issues 1 and 2.
- Confirmed the workspace version is 0.2.0 and that D-449 closed the rows
  v0.2.0 proved. Live register rows are PW-006 (`REQUIRED`) and PW-010
  (`READY`). `TRIGGER` and `WIZZY` rows stay stopped.
- Wrote `docs/roadmap/NEXT.md`, the v0.3.0 assignment index, with sizes and
  directories. Issue 1 is recorded as met by v0.2.0. Issue 2 has a Windows
  procedure and stays open because this environment cannot run it.
- Draft pull request: https://github.com/wizards-ecosystem/wizards-bedrock/pull/3
- Corrected current-facing status in `README.md`, `docs/014-storage.md`,
  `docs/017-pre-wizzy-hostile-surface-audit.md`, and `docs/releases/v0.1.0.md`.
  PW-580, PW-706, PW-571, and PW-578 are described as closed by v0.2.0 where
  the surrounding text was a current summary. Dated decision records were
  left dated.
- Added README "Start here" with the four public commands, channel `1.97.1`,
  and the statement that a successful build is not a boot.

## In progress

- Experimental register index under `experiments/`, outside the Cargo
  workspace.

## Next steps

- Add `experiments/register-index`, mark it non-authoritative, and run only
  that crate's `cargo test`.
- Update the pull request body.
- Do not run `cargo xtask ci`, QEMU, or the mutation sweep.
- Keep this file current on every push. Leave the PR as a draft.

## Decisions

- Issue 1 needs no source change. v0.2.0 already published the separate
  qualified release the issue describes. Closing the GitHub issue is a
  maintainer action; this stream does not write to GitHub issues.
- Issue 2 stays open. A Linux reading of the quickstart is not the clean
  Windows report the issue requires.
- `docs/roadmap/NEXT.md` is an assignment index. It does not add register
  rows and does not authorize `TRIGGER` work or a Rust userland.
- No security-vulnerability detail is added to the roadmap, the README, or
  the pull request. The audit stays identified as PW-006 and PW-010.
