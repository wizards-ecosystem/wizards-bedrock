# 005 — Code Conventions

How this repository is written. Adopted 2026-08-12 alongside the framekernel
crate split (D-21); the lint policy behind it is D-22.

The rule underneath all of these: **the compiler enforces what it can, and the
rest is written down here.** Where a convention could be a lint, it is one.

## Crate boundaries

| Crate | May write `unsafe`? | Purpose |
|---|---|---|
| `wost/` | **yes — and nowhere else does** | Privileged framework: boot handoff, port I/O, MMIO, CPU control, locking. The kernel's memory-safety TCB. |
| `kernel/` | no (`#![deny(unsafe_code)]`) | The kernel binary: drivers, console, logging, panic reporting, eventually scheduler and VFS. |
| `libs/` | no (`#![forbid(unsafe_code)]`) | Pure, hardware-free logic, host-testable. |
| `xtask/` | no | Host build tooling. Not part of the kernel. |

**What may go in `wost`:** an operation that cannot be expressed safely — a
raw instruction, a raw pointer, a hardware register. Nothing else. If a
function touches no hardware and holds no raw pointer, it does not belong in
`wost`; it belongs in `kernel/` or, better, in a `libs/` crate where it can be
unit-tested on the host.

**What `wost` owes its callers:** every abstraction it exports must make the
unsafe operation underneath it *checkable*, not merely *wrapped*.
`MmioRegion` bounds- and alignment-checks. `port::claim` records ownership in a
registry so exclusivity is verified rather than promised. `SpinLock::force_unlock`
takes a `PanicToken` — a value only the terminal panic path can obtain — so the
obligation is discharged by the type system instead of by a comment. Following
Asterinas (`002-architecture.md` pillar 5): these abstractions are designed in
from day one, never retrofitted.

`kernel/` uses `deny`, not `forbid`, for exactly one reason: `wost::entry!`
expands `#[allow(unsafe_code)]` boot glue into the binary crate, and `forbid`
cannot be lifted by an `allow` anywhere, including in a macro expansion. Crates
with no entry glue use `forbid`.

## Modules and layout

- One concept per file; no `mod.rs` unless the module has children.
- Architecture-specific code lives under `arch/<arch>/`, reached through
  `arch::current` so callers never name a concrete architecture.
- A module that names an external protocol or vendor crate should be the *only*
  one that does. `wost::boot::limine_glue` is the whole of the bootloader
  dependency; swapping bootloaders touches one file.
- In a binary crate, items are `pub(crate)`, not `pub` — `unreachable_pub`
  enforces it.

## Naming

- Conventional Rust throughout. The wizard theme lives in product-facing
  strings — banners, app names, shell personality — and never in identifiers
  (D-19).
- Spell things out: `registers`, `intensity`, `framebuffer`. Established
  systems jargon stays short: `uart`, `mmio`, `lba`, `bpp`, `esp`.
- Constants replace literals. A bare `0xC7` in a driver is a defect; a
  `fifo_control::ENABLE | fifo_control::CLEAR_RECEIVE | …` is the same byte
  and says what it means.
- Group related constants in a small private module (`register`, `line_status`)
  rather than prefixing names.
- Imports in three blocks, blank-line separated: `core`/`std`, external crates,
  then `crate`. This is convention, not configuration — rustfmt's
  `group_imports` is nightly-only and stable `cargo fmt` ignores it silently.

## Comments

**Few comments. A comment explains WHY, never WHAT.** If a comment restates
the code, delete one of them — usually the comment.

Exactly two kinds of prose are mandatory, and both are enforced:

- `// SAFETY:` before every `unsafe` block and `unsafe impl`, saying which
  precondition holds and why (`clippy::undocumented_unsafe_blocks`). One unsafe
  operation per block, so each comment covers exactly one thing
  (`clippy::multiple_unsafe_ops_per_block`).
- `# Safety` on every `pub unsafe fn`, stating the caller's obligation
  (`clippy::missing_safety_doc`).

Beyond that: `//!` on every module saying what it is for and what it assumes;
`///` on public items (`missing_docs`, at warn). `missing_docs_in_private_items`
is deliberately **not** enabled — it manufactures noise on every helper.

The comments worth writing are the ones recording a decision that the code
cannot show: why the divisor is 1, why the entry glue expands in the binary
crate, why a non-zero QEMU exit is not an error.

## Error handling

**Kernel and `wost`:**

- No `unwrap`, no `expect`, no panicking indexing on any path that runs at
  runtime. Return `Result` where a caller can do something, or an `Option`
  where absence is ordinary.
- `assert!` is for invariants that are genuinely unrecoverable, and its message
  says what was violated — a rejected bootloader base revision, a register
  index outside its block.
- Make total what can be made total. Glyph lookup falls back rather than
  unwrapping; a pixel outside the visible area is clipped rather than checked
  by the caller.
- Device errors are typed enums with a `Display` impl, not strings.

**xtask:** `anyhow` with `.context(...)` on **every** fallible call, phrased so
the message reads as "while doing X" when chained. No `unwrap`.

## Tests

`cargo test -p wizard-kernel` will never work: a `#![no_std] #![no_main]`
binary cannot host the `test` harness, and in-QEMU `#[test_case]` runners need
nightly's `custom_test_frameworks`, which D-01 rules out. So:

- Pure logic gets extracted to `libs/` crates carrying
  `#![cfg_attr(not(test), no_std)]`, where `cargo test` runs it on the host.
  `libs/pixel` is the worked example: colour, channel packing, and framebuffer
  offset arithmetic — the code that carried a live out-of-bounds bug while it
  was buried in the kernel. Add crates here for page-table bit packing,
  descriptor encoding, ELF parsing, ring index arithmetic, and the WizFS
  on-disk format as those arrive.
- A `libs/` crate goes in `default-members` so a bare `cargo test` covers it,
  and in `HOST_PACKAGES` in `xtask/src/paths.rs` so `cargo xtask ci` does too.
- `xtask` logic is unit-tested directly, in-module. Anything touching the
  filesystem is driven through a `Cursor` rather than a real file.
- Constructors that read the environment are split from pure ones —
  `Paths::new` takes everything explicitly, `Paths::discover` reads env and
  delegates. `std::env::set_var` is `unsafe` in edition 2024 and unsound across
  test threads, so tests never touch the environment.
- Hardware behaviour is verified by booting: `cargo xtask run` for the loop,
  a VMware boot as every phase's definition of done (D-18).

## Building

`cargo xtask ci` is the gate; it runs what CI runs. `cargo build --workspace`
does **not** work and cannot on stable — the kernel crates only build for
`x86_64-unknown-none` and `xtask` only for the host, and `package.forced-target`
is nightly. Bare cargo commands are scoped to the host half by
`default-members`; `cargo kcheck` / `kclippy` / `kbuild` reach the other half.

Never set the `RUSTFLAGS` environment variable in this repo: it replaces the
`[target.x86_64-unknown-none]` rustflags and silently produces a kernel without
`code-model=kernel`. Pass lint flags after `--`.

Set `CARGO_TARGET_DIR` if you want artifacts elsewhere; `build.target-dir` in a
cargo config is not honoured by xtask, which would need to parse `cargo metadata`
to see it.
