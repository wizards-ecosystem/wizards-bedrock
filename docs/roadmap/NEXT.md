# v0.3.0 assignment index

This file assigns work after the v0.2.0 qualified diagnostic release (D-449,
tag on `694061f`). It is an index for the next round of changes. The
[foundation register](../016-pre-wizzy-foundation.md) remains the only kernel
backlog. A row here does not admit work, close a row, or start a `TRIGGER`.

v0.2.0 shipped on 2026-10-28. It is a qualified diagnostic kernel image for
disposable virtual machines. It is not a foundation close, a desktop, a stable
ABI, or a bare-metal qualification. PW-006 and PW-010 are the live register
rows. Every `TRIGGER` and `WIZZY` row stays stopped.

Sizes are relative scope for one agent: **S** is a focused document or
single-crate edit, **M** crosses a few files in one directory, **L** needs a
qualification campaign or several subsystems. Directory lists are the files an
agent may edit. Leave every other directory alone.

## Open issues

### Issue 1 — met by v0.2.0

[Issue #1](https://github.com/wizards-ecosystem/wizards-bedrock/issues/1) asks
for one qualified diagnostic release after the v0.1.0 source preview, using
the register as the inventory and closing only rows whose proof criteria were
met. v0.2.0 is that release: `cargo xtask ci`, the 2,412-entry mutation sweep,
model checks, the fresh advisory review, both VMware selectors, and evidence
creation and verification all ran on `694061f`. The grade and Deep campaigns
are `deferred`, which D-425 allows. v0.1.0's tag was not moved. WZ-001 through
WZ-004 and the stopped Realm triggers were left outside the release.

No further source change belongs to this issue. A maintainer can close it.
The next qualified version is a new campaign under "What is left" in the
register, not a continuation of issue 1.

### Issue 2 — Windows GNU quickstart, still open

[Issue #2](https://github.com/wizards-ecosystem/wizards-bedrock/issues/2) asks
for a clean Windows report of the public quickstart. This tree was read on
Linux, so that report does not exist yet. The steps below are the acceptance
procedure. They edit nothing unless the report disagrees with the README.

Owner directory: `README.md`. A failure that is only a prose mismatch stays in
`README.md`. A failure that needs a build change belongs to `xtask/`,
`.cargo/`, or `rust-toolchain.toml`, which this index does not assign until
the report names the command and the error.

1. Use a fresh disposable Windows installation. Do not reuse a developer
   machine, a sibling Realm or Ink checkout, VMware, or a proprietary scanner.
2. Install rustup with the GNU host:
   `rustup-init -y --default-host x86_64-pc-windows-gnu`. Open a new terminal
   so `cargo` is on `PATH`.
3. Clone `https://github.com/wizards-ecosystem/wizards-bedrock.git` and check
   out the commit under test. Record `git rev-parse HEAD`.
4. Run only `cargo fetch --locked` and `cargo xtask build --release`.
5. Record the Windows version, `rustup show`, `rustc -vV`, and `cargo -vV`.
   The pinned channel in `rust-toolchain.toml` is `1.97.1`, with `rustfmt`,
   `clippy`, and the `x86_64-unknown-none` target. `rust-mingw` is omitted
   from that file because the component does not exist on other hosts; rustup
   installs it with the `x86_64-pc-windows-gnu` toolchain. Say whether that
   component was present after the first build.
6. Success is a compiled kernel. It is not a boot, a QEMU run, or
   qualification. Redact personal paths. Report the exact commands, the
   outcome of each, and every check that was not run.

The README now names that pinned channel and that limit. Issue 2 stays open
until a Windows report is attached to the issue.

## Priority 1 — live foundation rows

These two rows are what still keep the kernel foundation open after v0.2.0.
Do them before any phase 7–13 product work.

| Order | Item | Size | Directories | Notes |
|---:|---|---|---|---|
| 1 | PW-010, synchronize the hostile-surface census with the source | M | `docs/017-pre-wizzy-hostile-surface-audit.md` only, until a missing surface is a confirmed defect | Re-read every arithmetic, indexing, shift, and callee boundary the census claims. D-325 and D-341 already reject a stale `OPEN` label and an unreadable cross-cutting row. Update the census in place. Do not change a register state in this step. |
| 2 | PW-006, repeat the hostile-surface audit and record only reproduced defects | L | `docs/017-pre-wizzy-hostile-surface-audit.md`, then one of `kernel/`, `wost/`, `libs/`, `xtask/`, or `boot/` per confirmed defect | Re-attack hostile inputs, cleanup paths, safe `wost` callers, and release-only invariant failures. A new register row needs a concrete defect, broken invariant, missing proof, or named handoff consumer. The phase-close ADR in `docs/004-decision-log.md` is written after the evidence exists, and it links the audit, each break-test, and the remaining trigger-only rows. Historical boots are not new evidence. |

PW-006 implementation order, so parallel agents do not edit the same files:

1. One agent owns `docs/017-pre-wizzy-hostile-surface-audit.md` and produces a
   defect list. Each defect names one directory and the check that should go
   red.
2. After that list is committed, separate agents take `kernel/`, `wost/`,
   `libs/`, `xtask/`, and `boot/`. Each agent edits only its directory and
   only the defects assigned to it.
3. The docs agent then records proved rows in `docs/016-pre-wizzy-foundation.md`
   and the ADR in `docs/004-decision-log.md`. Unproved rows stay live.
4. Nobody starts `cargo xtask mutate`, `cargo xtask evidence`, or a VMware
   boot in parallel with those edits. v0.2.0 had to repeat its sweep once an
   edited index invalidated the report (D-449).

Public write-ups of this audit name the register row and the proof. They do
not add exploit detail.

## Priority 2 — the next qualified release

Run this only on a frozen commit after priority 1 has either closed its rows
or listed them as still live. The procedure is the register's "What is left"
section. Tag the qualified commit directly with a new version. Do not move
`v0.1.0` or `v0.2.0`.

| Order | Item | Size | Directories | Notes |
|---:|---|---|---|---|
| 3 | Freeze a candidate and run the D-425 gate on that exact commit | L | no feature edits; `xtask/` only if a qualification tool is itself defective | `cargo xtask ci`, `cargo xtask mutate`, `cargo xtask model-check`, `cargo xtask qualify-advisory`, both `cargo xtask vmware` selectors, and `cargo xtask qualify-vmware` / `cargo xtask qualify-vmware --shipped`. Both transcripts must show `iommu: translation on` before `nvme: up`. The grade and Deep campaigns enter the bundle only when their records name this commit; otherwise the manifest says `deferred`. |
| 4 | Create and verify evidence, then tag | M | version files only on the commit that will be tagged: `Cargo.toml` and the generated SBOM path `xtask/` already reads | `cargo xtask evidence` and `cargo xtask verify-evidence` on that same commit, `same-commit` relation, schema v11. The following commit closes rows the gate proved and lists every remaining live row. That following commit is not part of the evidence. |
| 5 | Publish the diagnostic release notes | S | `docs/releases/` | Match `docs/releases/v0.2.0.md`: exact commit, what ran, what stayed open, and the instruction to boot only in a disposable VM. Publish no benchmark that the gate did not record. |

The grade (`cargo xtask qualify-mutants-grade --run`) and Deep
(`cargo xtask qualify-security-deep --run`) campaigns are continuous against
`main`. They are not release blockers. They need a frozen tree, exclusive
machine time, and, for Deep, the owned authenticated session. Do not assign
them to an agent that is also editing source.

## Priority 3 — documentation that is still worth doing

| Order | Item | Size | Directories | Notes |
|---:|---|---|---|---|
| 6 | Attach the issue 2 Windows report, and correct the quickstart where the report disagrees | S | `README.md` | The procedure is above. A Linux or macOS result does not close the issue. PW-708 keeps a second hosted gate stopped. |
| 7 | Keep current-facing status in step with D-449 | S | `README.md`, `docs/014-storage.md`, `docs/017-pre-wizzy-hostile-surface-audit.md`, `docs/releases/` | Dated decision text stays dated. Summaries that still say PW-580, PW-706, PW-571, or PW-578 are waiting on VMware are stale: v0.2.0 closed them. |
| 8 | Point newcomers at one reading order | S | `README.md`, `docs/000-vision.md` | Vision, architecture, code conventions, the register, then this file. Phase docs stay historical ledgers. |

## Priority 4 — later kernel features, not this release's default

Start these only after priority 1 is recorded and only when the register
admits them. Each one is a new design doc before code.

| Item | Size | Directories | Why it waits |
|---|---|---|---|
| Phase 8 kernel NIC and async socket admission | L | `docs/`, then `wost/`, `kernel/`, and a new `libs/` crate for hardware-free packet logic | Kernel pieces stay Rust. First-party target services stay Wizzy and belong to Realm. No socket ABI without a named caller. |
| Phase 12 SMP bring-up | L | `docs/`, `wost/`, `kernel/` | D-154 and the per-CPU triggers wait for that phase. The per-CPU seam already exists. |
| Phase 13 WizFS on-disk format | L | a design doc first; `libs/wizfs-format` only when the phase starts | The crate is named and absent on purpose (D-73). An experimental sketch outside `libs/` does not admit it. |
| AMD-Vi backend | L | `libs/vtd` consumers in `wost/` | PW-711 fires on a qualified AMD host. |
| Non-identity IOVA maps | M | `wost/`, `libs/` | PW-712 fires when a caller must hide the physical map from a device. |

## Stopped — do not assign

These are in the register so they are not rediscovered as feature ideas.

| Row or phase | Size if it ever opens | Directory it would touch | Trigger |
|---|---|---|---|
| PW-601 consumer scaffold | L | Realm, not this repository, except a kernel ABI change the pinned artifact forces | Pinned draft artifact and runtime seam |
| PW-602 gate opening | M | `docs/015-wizzy-userland.md` for the Bedrock admission record | WZ-001 through WZ-004 together at one Ink revision, then a Realm decision |
| WZ-001 through WZ-004 | L | `wizards-ink`, external | Ink owns the compiler gate |
| PW-700 frame and message quotas | M | `kernel/`, `libs/frame-space` | First user spawn, map, or channel-send route |
| PW-701 dynamic stacks and `MemoryObject` | L | `kernel/`, `wost/`, `libs/stack-layout` | Post-gate measurement of a real runtime |
| PW-702 usercopy unmap race | M | `wost/` | A second thread that can revoke the copied range |
| PW-703 dynamic stack-token extent | S | `libs/stack-layout`, `wost/` | Stacks that grow or shrink |
| PW-704 page cache, writeback, interrupt NVMe | L | `kernel/`, `libs/fat32`, `libs/nvme` | A pinned Wizzy storage workload |
| PW-705 dependency and toolchain bumps | S | root manifests, `xtask/` | A named host, dependency, or Wizzy caller forces the pin |
| PW-707 new leaf-guard callbacks | M | `wost/`, `kernel/` | New caller-supplied work under a leaf guard |
| PW-708 non-Windows hosted gate | M | `.github/`, `xtask/` | A second active contributor or a decision to support that host |
| PW-531 transcript challenge binding | M | `xtask/`, `kernel/` | A guest-readable per-generation challenge or attestation |
| Phase 7 product `init`, shell, desktop | L | Realm | Compiler-readiness gate, currently closed (D-183) |
| Rust fallback userland | — | none | Declined (D-183) |
| Own bootloader | — | none | No unmet handoff (D-219) |

## Suggested next-round split

Use one owner per directory. The audit writer finishes the defect list before
the code owners start.

| Owner | Directories | Starts with |
|---|---|---|
| A | `docs/017-pre-wizzy-hostile-surface-audit.md` | PW-010 census, then the PW-006 defect list |
| B | `kernel/` | Defects A assigns to `kernel/` |
| C | `wost/` | Defects A assigns to `wost/` |
| D | `libs/` | Defects A assigns to `libs/` |
| E | `xtask/` | Witnesses for those defects; no full sweep until the tree is frozen |
| F | `boot/` | Only a defect A assigns to `boot/` |
| G | `README.md`, `docs/roadmap/`, `docs/releases/` | Issue 2's report and the release notes of the next tag |
| H | `Cargo.toml`, `.github/`, `rust-toolchain.toml` | Only a pin change PW-705's trigger has fired, or the version bump on the commit that will be tagged |

`models/` changes only with `cargo xtask model-check` in the frozen
qualification, or when a ring-protocol invariant actually changes. `fixtures/`
changes only when the ABI transcription changes and its existing checker is
updated in the same change.

## Reading order for a newcomer

1. [README](../../README.md) "Start here", then [architecture](../002-architecture.md).
2. [Code conventions](../005-code-conventions.md) before editing.
3. The subsystem doc in `docs/` for the area being changed.
4. [Foundation register](../016-pre-wizzy-foundation.md) for what is live.
5. This file for who may edit which directory.
6. [v0.2.0 release scope](../releases/v0.2.0.md) for what the qualified image
   actually demonstrated.

`experiments/` is outside the workspace and outside this backlog. A crate
there can illustrate a stopped or later idea. It is not kernel code, not a
register row, and not evidence.
