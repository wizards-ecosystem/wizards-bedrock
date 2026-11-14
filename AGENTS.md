# Working in this repository

This is The Wizard's Bedrock, a Rust kernel built design-first: **docs in `docs/` come before
code**, and every choice is recorded with its alternatives and rationale in a
file in `docs/decisions/`, indexed by `docs/004-decision-log.md`. Read the relevant
doc before changing a subsystem, and add a decision file when you make a decision
worth remembering.

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
the authoritative finite kernel foundation register. Every new item needs
the defect, invariant, proof gap, handoff consumer or trigger the register
requires; `TRIGGER` and post-gate rows are stopped work, not invitations.

**Read [docs/005-code-conventions.md](docs/005-code-conventions.md) before
writing code.** It is the authority on crate boundaries, naming, error
handling, testing, and the comment policy. Three things from it that are easy
to get wrong:

- `wost/` is the **only** crate that may write `unsafe`. `kernel/` carries
  `#![deny(unsafe_code)]`. If something needs a raw pointer or instruction, the
  safe abstraction goes in `wost`. Do not reach for `#[allow(unsafe_code)]`.
  There are exactly three valid reasons to write `unsafe` (D-41): a
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
recorded, a subsystem that boots. Do not leave a green gate uncommitted. Each
commit should be a thing that works: `cargo xtask ci` passes at every commit, except for the
bounded first source-preview launch exception below.

Match the existing log's voice: a short declarative line saying what changed
and what it bought, not a changelog entry. `git log --oneline` shows the
register.

**Work on `main`, and push as you commit.** The gate at every commit is what
keeps `main` green, so a branch would only defer the merge that is coming
anyway. Do not create a branch unless Isaac asks.

## Commands

`cargo xtask ci` remains the ordinary gate. Run it before claiming ordinary
implementation work is done. D-441 permits only the first v0.1.0 launch's
documentation, community infrastructure and source-preview packaging commits
to use its bounded source-preview checks instead: disclosure/license review,
whitespace, fmt, supply-chain, vendor and release compile on a clean candidate, with a
20-minute aggregate automated budget. This is not permission to skip checks for kernel
changes or future qualified releases. Publish v0.1.0 only after those checks
pass; disclose every unrun qualification and distribute no new kernel/VM image
or evidence bundle. Its tag is immutable. D-26 permits only the lightweight
read-only hosted Source preview checks; those do not replace local CI.
Outside contributors use [CONTRIBUTING.md](CONTRIBUTING.md), including forks
and PRs; the direct-to-main rule above is the owner's workflow.

`cargo xtask help` lists every task. The README's [Development](README.md#development)
section covers the two
non-obvious rules (`cargo build --workspace` cannot work here; never set
`RUSTFLAGS`). Read it before reaching for a bare cargo command.

Hardware behaviour is verified by booting, not by tests: `cargo xtask run`
for QEMU, and a VMware boot as every phase's definition of done (D-18). The
VMware boot reaches SMEP/SMAP/UMIP, the compacted
`XSAVEC` path, a real x2APIC and a processor that reports `MD_CLEAR`, none of
which the gate's QEMU does.

Five supported tasks are **not** in ordinary `ci`: `debug`, `baseline`,
`vmware`, `qualify-vmware` and `model-check`. `vmware` generates each selected
machine; `qualify-vmware` validates its fresh serial transcript after the human
boot and keeps no record. Model checking runs when a model changes. These tasks
need interactive or external tools, and putting them in the ordinary gate would
make a missing tool look like a source failure.

A release (D-454) is `cargo xtask ci` green, `cargo xtask evidence` and
`cargo xtask verify-evidence` on the exact commit, a VMware boot of both images
and a person reading the release diff. `cargo mutants` over changed library code
and fuzzing are laptop work that never blocks a commit.

## Testing, and the rule that matters most

The suite is small on purpose (D-454). It has six tiers: one boot smoke that
also scans the serial log; syscall and ABI behaviour; isolation and security;
a few edge tests per hostile-input parser; a regression for each fixed bug; and
about ten tooling smoke tests. Hardware-free logic goes to a `libs/` crate and
runs under `cargo test`; anything needing a real machine goes to
`kernel/src/selftest.rs` and runs under `cargo xtask test-boot`.

Do not add a test that restates the code, asserts a constant, a table or a
layout, or reads source text: a source rule is a lint, a `deny` or `forbid`
attribute, or a type. A change adds at most one focused test, and only for a
real bug. A check that cannot fail is worth less than no check, so write each
one from the claim it makes. `cargo mutants` runs on the laptop on demand;
nothing requires or records it.

Doctests run in `libs/` crates and nowhere else (D-41): those carry
`#![cfg_attr(not(test), no_std)]` and build for the host, so examples there are
executable checks. `wost` and `kernel` are cross-compiled, so examples in them
would rot unrun: leave them out.
