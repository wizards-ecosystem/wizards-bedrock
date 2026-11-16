# Working in this repository

The Wizard's Bedrock is a Rust kernel for x86_64 and UEFI. This repository is the kernel only:
hardware, memory, scheduling, storage and the native capability ABI.

## Read first

- `docs/architecture.md`: how the kernel works today.
- `docs/conventions.md`: crate boundaries, naming, errors, comments, tests and doc style.
- `docs/roadmap.md`: the order of work and where the code stands against it.
- `docs/decisions.md`: the decisions that still govern the kernel.
- `docs/statements.md`: what must be true of each core piece; code, audits and proofs work against it.

## Crates and unsafe

| Crate | `unsafe` | Holds |
|---|---|---|
| `wost/` | yes | instructions, registers, raw pointers and locks, behind safe APIs |
| `witness/rt/` | yes, beside `wost` | `_start`, the call gate, ring access and probes for ring 3 witnesses |
| `kernel/` | no (`#![deny(unsafe_code)]`) | the kernel binary |
| `witness/ring3/` | no (`#![deny(unsafe_code)]`) | the ring 3 witness program |
| `libs/*` | no (`#![forbid(unsafe_code)]`) | hardware-free `no_std` logic, tested on the host |
| `xtask/` | no (`#![forbid(unsafe_code)]`) | build, image, QEMU, VMware and the gate |

- Write `unsafe` only in `wost` and `witness/rt`, and only for a novel abstraction, a measured
  performance need, or FFI and the platform. Never add `#[allow(unsafe_code)]` anywhere else.
- Each `unsafe` block carries `// SAFETY:` and holds one unsafe operation. Each `pub unsafe fn`
  carries `# Safety`.
- A safe `wost` export must not let any caller reach undefined behaviour.
- Arithmetic that needs no hardware goes in a `libs/` crate; `wost` keeps the lock and the
  instruction.
- Add an API with its first caller, not before.
- Comments are near zero: `// SAFETY:`, `# Safety`, and one line for a constraint the code cannot
  show. A comment never cites a decision or a doc.

## Build and check

```
cargo xtask build --release    compile the kernel and check the linked image
cargo xtask run                boot the shipped image in QEMU
cargo xtask test-boot          boot the self-test kernel and read its verdict
cargo xtask ci                 the gate
cargo xtask help               every task
```

- Never set `RUSTFLAGS`. It replaces the kernel code-model flags in `.cargo/config.toml`, and the
  kernel then links wrong with no error.
- `cargo build --workspace` does not work: the kernel crates build only for `x86_64-unknown-none`.
  Bare `cargo check`, `clippy` and `test` cover the host crates; `cargo kcheck`, `kclippy` and
  `kbuild` reach the kernel.
- `model-check`, `vmware`, `qualify-vmware` and `debug` are outside `ci` because each needs a tool
  or a person. Run `model-check` when a file in `models/` changes.
- Hardware behaviour is verified by booting, not by tests. QEMU is the loop; a VMware boot of the
  self-test and shipped machines closes a phase.
- A release is `cargo xtask ci` green on the exact commit, a VMware boot of both generated machines
  accepted by `cargo xtask qualify-vmware`, and a person reading the release diff.

## Commits

- Work on `main` and push as you commit. Create a branch only when the owner asks. Outside
  contributors use forks and pull requests (`CONTRIBUTING.md`).
- Commit at each working checkpoint. `cargo xtask ci` runs once on the tip of each push, not on
  every commit; when it fails, bisect the push.
- The subject is one short declarative line that says what changed and what it bought.
  `git log --oneline` shows the voice.
- No AI co-author trailers or attribution.
- A design choice with real alternatives gets a new entry in `docs/decisions.md`. Never rewrite a
  landed decision; amend it with a new one.

## Tests

- The suite is minimal. Keep a test only if it pins a contract a user could hit or a fixed real
  bug with non-trivial logic.
- A change adds at most one focused test, and only when it fixes a real bug. Prefer extending an
  existing test to adding a file.
- Never test prose, constants, tables, layouts, wiring or source text; what must hold of the source
  is a lint, a deny or a type.
- A flaky or slow check is deleted, not fixed.
- Builds and test tiers run on the laptop, never on the desktop.

Bedrock's tiers and where each check goes are in `docs/conventions.md`. `cargo mutants` and fuzzing
are on-demand laptop work that nothing requires or records.

## Docs

- Docs never fail a build: no test, script or CI step checks prose against code.
- Update the docs a change makes false in the same change; at a release, read the docs against the
  code and fix what drifted.
