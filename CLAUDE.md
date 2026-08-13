# Working in this repository

This is a from-scratch OS built design-first: **docs in `docs/` come before
code**, and every choice is recorded in `docs/004-decision-log.md` with its
alternatives and rationale. Read the relevant doc before changing a subsystem,
and add an ADR when you make a decision worth remembering.

**Read [docs/005-code-conventions.md](docs/005-code-conventions.md) before
writing code.** It is the authority on crate boundaries, naming, error
handling, testing, and the comment policy. Two things from it that are easy to
get wrong:

- `wost/` is the **only** crate that may write `unsafe`. `kernel/` carries
  `#![deny(unsafe_code)]`. If something needs a raw pointer or instruction, the
  safe abstraction goes in `wost` — do not reach for `#[allow(unsafe_code)]`.
- **Few comments.** Comments explain *why*, never *what*. The mandatory prose
  is `// SAFETY:` on unsafe operations and `///` on public items; the lints
  enforce both.

## Commands

`cargo xtask ci` is the gate — run it before claiming work is done. `cargo
xtask help` lists every task.

`cargo build --workspace` does not work on stable; use `cargo kcheck`,
`cargo kclippy`, `cargo kbuild` for the kernel crates. Never set `RUSTFLAGS` —
it silently replaces the kernel's link flags. Both are explained in the README.

Hardware behaviour is verified by booting, not by tests: `cargo xtask run`
for QEMU, and a VMware boot as every phase's definition of done (D-18).
