# Working in this repository

This is a from-scratch OS built design-first: **docs in `docs/` come before
code**, and every choice is recorded in `docs/004-decision-log.md` with its
alternatives and rationale. Read the relevant doc before changing a subsystem,
and add an ADR when you make a decision worth remembering.

**Read [docs/005-code-conventions.md](docs/005-code-conventions.md) before
writing code.** It is the authority on crate boundaries, naming, error
handling, testing, and the comment policy. Three things from it that are easy
to get wrong:

- `wost/` is the **only** crate that may write `unsafe`. `kernel/` carries
  `#![deny(unsafe_code)]`. If something needs a raw pointer or instruction, the
  safe abstraction goes in `wost` — do not reach for `#[allow(unsafe_code)]`.
  There are exactly three valid reasons to write `unsafe` at all (D-41): a
  novel abstraction, a measured performance need, or FFI/the platform.
- **Few comments.** Comments explain *why*, never *what*. The mandatory prose
  is `// SAFETY:` on unsafe operations and `///` on public items; the lints
  enforce both. One unsafe operation per block.
- **Pure arithmetic goes to a `libs/` crate**, where `cargo test` can reach it
  on the host. If a bound needs arithmetic to check, the arithmetic moves and
  `wost` keeps only the lock and the instruction — `port_space::PortSpace` and
  `frame_space::FrameSpace` are the worked examples.

## Commits

**Commit at meaningful checkpoints as you go** — a finished slice, a decision
recorded, a subsystem that boots. Do not save everything for the end of a
session, and do not leave a green gate uncommitted. Each commit should be a
thing that works: `cargo xtask ci` passes at every commit.

Match the existing log's voice — a short declarative line saying what changed
and what it bought, not a changelog entry. `git log --oneline` shows the
register. Branch first if you are on `main` and the change is substantial.

## Commands

`cargo xtask ci` is the gate — run it before claiming work is done. `cargo
xtask help` lists every task. The README's Development section covers the two
non-obvious rules (`cargo build --workspace` cannot work here; never set
`RUSTFLAGS`) — read it before reaching for a bare cargo command.

Hardware behaviour is verified by booting, not by tests: `cargo xtask run`
for QEMU, and a VMware boot as every phase's definition of done (D-18).

## Testing, and the rule that matters most

Every change lands with the check that proves it. Where the check goes is a
property of the code, not a preference: hardware-free logic goes to a `libs/`
crate and runs under `cargo test`; anything needing a real machine goes to
`kernel/src/selftest.rs` and runs under `cargo xtask test-boot`.

**Break every new check once and watch the harness go red** (D-28). This is not
ceremony. A check written in slice 1 of Phase 3 passed with the code it tested
deleted, because two early returns made its failure path unreachable — and
nothing but breaking it would have found that. A check that cannot fail is
worth less than no check, because it is also a claim.

Doctests run in `libs/` crates and nowhere else (D-41): those carry
`#![cfg_attr(not(test), no_std)]` and build for the host, so examples there are
executable checks. `wost` and `kernel` are cross-compiled, so examples in them
would rot unrun — leave them out.
