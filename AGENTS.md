# Working in this repository

This is The Wizard's Bedrock, a Rust kernel built design-first: **docs in `docs/` come before
code**, and every choice is recorded in `docs/004-decision-log.md` with its
alternatives and rationale. Read the relevant doc before changing a subsystem,
and add an ADR when you make a decision worth remembering.

## Current roadmap gate

D-183 fixes the language boundary: in-kernel code stays Rust, and every
feasible first-party target-side program, service, library, tool and application
is Wizzy. Realm owns the canonical compiler-readiness gate and product plan under D-366.
`docs/015-wizzy-userland.md` owns the kernel handoff and records Bedrock's
last admitted compiler evidence as **CLOSED**. While closed, do not start Phase 7 product work or
build a Rust fallback userland. Work is in scope only when it is kernel
correctness, a language-neutral prerequisite with a concrete future Wizzy
caller, or specification and validation of the versioned compiler handoff.
Realm owns all target-side product implementation. Consumer scaffolding remains stopped until PW-601 has both a pinned draft
artifact and a runtime seam; WZ-001 through WZ-004 are externally owned
`wizards-ink` blockers, not Bedrock implementation work.

D-184 makes [docs/016-pre-wizzy-foundation.md](docs/016-pre-wizzy-foundation.md)
the sole finite kernel backlog while that gate is closed. Every new item needs
the defect, invariant, proof gap, handoff consumer or trigger the register
requires; `TRIGGER` and post-gate rows are stopped work, not invitations.

**Read [docs/005-code-conventions.md](docs/005-code-conventions.md) before
writing code.** It is the authority on crate boundaries, naming, error
handling, testing, and the comment policy. Three things from it that are easy
to get wrong:

- `wost/` is the **only** crate that may write `unsafe`. `kernel/` carries
  `#![deny(unsafe_code)]`. If something needs a raw pointer or instruction, the
  safe abstraction goes in `wost`. Do not reach for `#[allow(unsafe_code)]`.
  There are exactly three valid reasons to write `unsafe` at all (D-41): a
  novel abstraction, a measured performance need, or FFI/the platform.
- **Few comments.** Comments explain *why*, never *what*. The mandatory prose
  is `// SAFETY:` on unsafe operations and `///` on public items; the lints
  enforce both. One unsafe operation per block.
- **Pure arithmetic goes to a `libs/` crate**, where `cargo test` can reach it
  on the host. If a bound needs arithmetic to check, the arithmetic moves and
  `wost` keeps only the lock and the instruction: `port_space::PortSpace` and
  `frame_space::FrameSpace` are the worked examples.

## Commits

**Commit at meaningful checkpoints as you go**: a finished slice, a decision
recorded, a subsystem that boots. Do not save everything for the end of a
session, and do not leave a green gate uncommitted. Each commit should be a
thing that works: `cargo xtask ci` passes at every commit.

Match the existing log's voice: a short declarative line saying what changed
and what it bought, not a changelog entry. `git log --oneline` shows the
register.

**Work on `main`, and push as you commit.** There are no feature branches:
the gate at every commit is what keeps `main` green, so a branch would only
defer the merge that is coming anyway. Do not create one unless Isaac asks.

## Commands

`cargo xtask ci` is the gate. Run it before claiming work is done. `cargo
xtask help` lists every task. The README's [Development](README.md#development)
section covers the two
non-obvious rules (`cargo build --workspace` cannot work here; never set
`RUSTFLAGS`). Read it before reaching for a bare cargo command.

Hardware behaviour is verified by booting, not by tests: `cargo xtask run`
for QEMU, and a VMware boot as every phase's definition of done (D-18). The
VMware boot is not decoration: it reaches SMEP/SMAP/UMIP, the compacted
`XSAVEC` path, a real x2APIC and a processor that reports `MD_CLEAR`, none of
which the gate's QEMU does.

**`cargo xtask mutate` is how D-28 is kept true over time.** Breaking a check
by hand happens once, when it is written; a check that *stops* being able to
fail is invisible from then on. The task applies known defects and requires a
check to go red for each. Add a mutation whenever you add a check. `cargo
test` verifies every mutation still matches its source, so a rotted one is
caught in milliseconds even though each machine mutation still costs a boot.

Nine supported tasks are deliberately **not** in ordinary `ci`: `debug`,
`baseline`, `vmware`, `qualify-vmware`, `model-check`, `mutate` and
`qualify-advisory`, plus the multi-day `qualify-mutants-grade --run` and the
owned `qualify-security-deep --run` workflow (with diagnostic-only
`--begin`/`--verify`). `vmware` generates each selected machine;
`qualify-vmware` validates its fresh serial transcript after the human boot.
VMware, model checking, the complete mutation sweep, the independently owned
mutation grade, an owned authenticated Deep security scan and the fresh RustSec
review are phase-close qualifications whose records enter offline evidence.
Deep's `--begin` and imported `--verify` actions create no record; only the
pinned live MCP session can create PASS (D-406). These tasks need
interactive/external tools, current network data or much longer machine runs,
and putting them in the ordinary gate would make a missing tool look like a
source failure.

## Testing, and the rule that matters most

Every change lands with the check that proves it. Where the check goes is a
property of the code, not a preference: hardware-free logic goes to a `libs/`
crate and runs under `cargo test`; anything needing a real machine goes to
`kernel/src/selftest.rs` and runs under `cargo xtask test-boot`.

**Break every new check once and watch the harness go red** (D-28). This is not
ceremony. A check written in slice 1 of Phase 3 passed with the code it tested
deleted, because two early returns made its failure path unreachable, and
nothing but breaking it would have found that. A check that cannot fail is
worth less than no check, because it is also a claim.

Doctests run in `libs/` crates and nowhere else (D-41): those carry
`#![cfg_attr(not(test), no_std)]` and build for the host, so examples there are
executable checks. `wost` and `kernel` are cross-compiled, so examples in them
would rot unrun, so leave them out.
