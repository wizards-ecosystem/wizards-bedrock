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

## In progress

- README and current-facing docs still describe PW-580 and PW-706 as open.
  Those rows are `DONE`. The status correction is next.

## Next steps

- Correct README status, add a newcomer "Start here", and point the pinned
  toolchain at channel `1.97.1`.
- Correct current-facing sentences in `docs/014-storage.md`,
  `docs/017-pre-wizzy-hostile-surface-audit.md`, and the v0.1.0 release note.
- Prototype one experimental crate under `experiments/`, outside the Cargo
  workspace, that indexes the foundation register for the next assignment
  round. Mark it non-authoritative.
- Run only that crate's `cargo test`. Do not run `cargo xtask ci`, QEMU, or
  the mutation sweep.
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
