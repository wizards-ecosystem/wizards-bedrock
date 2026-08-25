# 005 — Code Conventions

How this repository is written. Adopted 2026-08-12 alongside the framekernel
crate split (D-21); the lint policy behind it is D-22.

The default standard is the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/),
with the [Rust for Linux coding guidelines](https://docs.kernel.org/rust/coding-guidelines.html)
taking precedence where they are more specific about kernel work (D-27). This
doc records only what those leave open, what this project decides differently,
and the checklist items worth calling out. `C-XXX` references below point at the
API Guidelines checklist.

The rule underneath all of these: **the compiler enforces what it can, and the
rest is written down here.** Where a convention could be a lint, it is one.

## Crate boundaries

| Crate | May write `unsafe`? | Purpose |
|---|---|---|
| `wost/` | **yes — and nowhere else does** | Privileged framework: boot handoff, port I/O, MMIO, CPU control, locking. The kernel's memory-safety TCB. |
| `kernel/` | no (`#![deny(unsafe_code)]`) | The kernel binary: drivers, console, logging, panic reporting, scheduler, and eventually VFS. |
| `libs/` | no (`#![forbid(unsafe_code)]`) | Pure, hardware-free logic, host-testable. |
| `xtask/` | no | Host build tooling. Not part of the kernel. |

**What may go in `wost`:** an operation that cannot be expressed safely — a
raw instruction, a raw pointer, a hardware register. Nothing else. If a
function touches no hardware and holds no raw pointer, it does not belong in
`wost`; it belongs in `kernel/` or, better, in a `libs/` crate where it can be
unit-tested on the host.

That test is a judgement, so it is backed by a closed list. **The only valid
reasons to write `unsafe` are** (Pragmatic Rust Guidelines, D-41):

1. **a novel abstraction** the language cannot express — a frame type, an
   allocator, a page table;
2. **performance**, where a checked form exists and measurably costs too much;
3. **FFI or the platform** — an instruction, a register, a bootloader ABI.

Anything else is a defect, not a trade-off. Reason 2 is the one that needs a
number attached before it counts, and so far nothing in this kernel has used
it.

**The standard every `wost` export is held to** is the Rustonomicon's
sound-wrapper principle: *safe code must not be able to cause undefined
behaviour through this abstraction, however it is called.* Not "is the unsafe
block correct" but "can any caller, including a wrong one, reach UB". That is
the same demand the paragraph below makes in different words, and it is the
question to ask of a new export before its `# Safety` section is written.

The Unsafe Code Guidelines Reference is **not** an authority here. It
describes itself as largely historical and advisory, and documents open
questions alongside settled rules; cite the Nomicon or the reference, not it.

**No speculative API.** Do not add an operation because a subsystem will
plausibly want it later. This matters most in `wost`, where an unused
`pub unsafe fn` carries a safety contract nobody can check — `enable_interrupts`
was written and then removed for exactly this reason: its contract has to talk
about an IDT, and until one exists the contract is a guess. Write it alongside
its first caller, when the obligation is real and testable. The exception is
completing a primitive that is incoherent in halves: `PortValue` covers 8, 16,
and 32 bits because a sealed trait with one impl is not an abstraction.

**What `wost` owes its callers:** every abstraction it exports must make the
unsafe operation underneath it *checkable*, not merely *wrapped*.
`MmioRegion` bounds- and alignment-checks. `port::claim` records ownership in a
registry so exclusivity is verified rather than promised — over the span an
access *reaches*, not the port it names, which is the distinction Phase 2 got
wrong and D-39 fixed. A spin lock's recovery operation and its proof value are
crate-private, and only the non-returning panic boundary can reach them. A
safe caller cannot manufacture recovery authority or resume after violating a
live guard. Following Asterinas
(`002-architecture.md` pillar 5): these abstractions are designed in from day
one, never retrofitted.

A bound stated in prose is not a bound. Where the checkable form needs
arithmetic, the arithmetic goes to a `libs/` crate and `wost` keeps only the
lock and the instruction — `port_space::PortSpace` under
`arch::x86_64::port` is the pattern to copy.

**The panic path never blocks on a kernel lock.** A fault is not masked by the
interrupt flag, so any lock can be held by the code a panic interrupted.
`wost` reclaims only its own internal locks, inside a terminal function-pointer
boundary that cannot capture a live guard. Kernel reporting uses `try_lock` and
may omit a channel whose lock is busy. A new panic reporter that waits for a
kernel lock is a deadlock waiting for its first panic.

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
- **Never repeat a namespace inside an identifier** (Rust for Linux). The module
  path already carries it: `register::LINE_STATUS`, not `UART_REGISTER_LINE_STATUS`.
  Group related constants in a small private module rather than prefixing names.
- Casing per RFC 430, and `as_` / `to_` / `into_` used with their standard
  meanings (C-CASE, C-CONV).
- Consistent word order across an API (C-WORD-ORDER): verb then noun, and the
  same verb for the same operation — `fill_rows` / `copy_rows`, `write_u32` /
  `read_u32`. The example here used to be `write_pixel` / `read_pixel`; there
  is no `read_pixel` and there is not meant to be, because framebuffer memory
  is uncached and reading it is what the back buffer exists to avoid.
- **Arguments convey meaning through types, not `bool`** (C-CUSTOM-TYPE).
  `fmt(paths, true)` says nothing; `fmt(paths, FmtMode::Check)` says everything.
  This applies hardest to two adjacent parameters of the same type — hence
  `Palette` rather than a `(background, foreground)` pair the compiler would
  happily let you swap.
- Imports in three blocks, blank-line separated: `core`/`std`, external crates,
  then `crate`. This is convention, not configuration — rustfmt's
  `group_imports` is nightly-only and stable `cargo fmt` ignores it silently.
  Rust for Linux's one-item-per-line import layout is **not** adopted: it
  contradicts the rustfmt defaults this project formats with.

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

The distinction matters and is easy to blur (Rust for Linux): a `# Safety`
section states the *precondition a caller must meet*; a `// SAFETY:` comment
states *why this particular call meets it*. One is a contract, the other is
evidence.

Beyond that: `//!` on every module saying what it is for and what it assumes;
`///` on public items, whose first line is a single sentence saying what the
item does (`missing_docs`, at warn); `# Panics` on anything that can panic
(C-FAILURE). `missing_docs_in_private_items` is deliberately **not** enabled —
it manufactures noise on every helper.

C-EXAMPLE is adopted **only in `libs/`**, and the split is a fact about where
doctests can run rather than a preference (D-41). The floor is one runnable
example per `libs/` crate — on the module doc or its central item — and all
current crates carry one; more go on any item whose encoding an example states
better than prose. Not "an
example on every item", which the rule used to say and no crate has ever
done. `wost` and `kernel` are `no_std` and cross-compiled, so their doctests
cannot execute and an unrunnable example rots unnoticed. A `libs/` crate
carries `#![cfg_attr(not(test), no_std)]` precisely so `cargo test` builds it
for the host — where its doctests *do* compile and run, under the bare
`cargo test` that `default-members` already scopes. There an example is an
executable check, and for an encoding API it is the clearest statement of the
encoding there is: `libs/x86-paging`'s entry bits are the worked case.

Two rustdoc conventions that go with it:

- The first line is the module listing and the search result, so it is one
  sentence, ends in a period, and stays under about fifteen words.
- Sections appear in the conventional order — summary, body, `# Examples`,
  `# Errors`, `# Panics`, `# Safety` — because that is the order every
  experienced reader scans for.
- `#[doc(hidden)]` on items that are `pub` only for a technical reason.
  `boot::__macro` is the case: `entry!` needs those names in scope in the
  calling crate, and nobody should read them as API.

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
- Error types are enums, never strings, and they are **well-behaved**
  (C-GOOD-ERR): `Debug`, `Display`, and `core::error::Error`, which is available
  in `core` since Rust 1.81 and needs no crate. `Display` writes a lowercase
  clause with no trailing period, so it reads correctly inside a sentence —
  `"no serial log: no controller answered the loopback probe"`.
- Error enums carry `#[non_exhaustive]`. A new variant is then not a breaking
  change, and every `match` on one is written to survive growth.
- **Types implement the common traits eagerly** (C-COMMON-TRAITS, C-DEBUG):
  `Debug` on everything public, enforced by `missing_debug_implementations`, plus
  `Clone`/`Copy`/`PartialEq`/`Eq` on plain data. `Debug` matters more here than
  in most Rust: serial output is the only debugger there is. A `Debug` impl must
  never block — `SpinLock`'s uses `try_lock` and prints `<locked>` rather than
  spinning, because the context most likely to print one is the panic handler.

**xtask:** `anyhow` with `.context(...)` on **every** fallible call, phrased so
the message reads as "while doing X" when chained. No `unwrap`.

## Tests

**Every change lands with the check that proves it.** There are exactly two
places a check can go, and which one is a property of the code, not a
preference:

| The thing under test | Where it goes | How it runs |
|---|---|---|
| Arithmetic, bit layout, parsing — no hardware | a `libs/` crate | `cargo test` |
| Anything needing a real machine | `kernel/src/selftest.rs` | `cargo xtask test-boot` |

If a check fits neither, that is usually a sign the logic should be split so
that the interesting half fits the first row. `libs/pixel` exists because the
framebuffer's arithmetic was worth testing and its hardware access was not.

`cargo test -p wizard-kernel` will never work: a `#![no_std] #![no_main]`
binary cannot host the `test` harness, and in-QEMU `#[test_case]` runners need
nightly's `custom_test_frameworks`, which D-01 rules out. So:

- Pure logic gets extracted to `libs/` crates carrying
  `#![cfg_attr(not(test), no_std)]`, where `cargo test` runs it on the host.
  `libs/pixel` is the worked example: colour, channel packing, and framebuffer
  offset arithmetic — the code that carried a live out-of-bounds bug while it
  was buried in the kernel. `libs/x86-tables` is the second: GDT, IDT and TSS
  encoding, where the host test is the only thing that can check a bit position
  against the manual. `libs/x86-apic` and `libs/x86-pit` follow on the same
  grounds, and `libs/ps2-keys` for the scancode state machine.
  `libs/port-space` is a particularly clear case of *why*: the I/O port
  reservation bitmap sat inside `wost`, where nothing could test it, and it
  spent Phase 2 recording one port per claim however wide the access it
  authorised (D-39). Page-table bit packing has since become
  `libs/x86-paging`; add crates on the same pattern for ELF parsing, ring index
  arithmetic, and the WizFS on-disk format as those arrive.
- **The gate runs over a configuration matrix, not a codebase.** A cargo
  feature is a different crate graph, and a lint that fires in one need not
  fire in the other: `cargo xtask ci` covers the host packages, the kernel
  packages, and the kernel packages *again* with `selftest` on. Adding a
  feature means adding a pass — see the `PASSES` table in `xtask/src/tasks.rs`
  and the D-22 amendment for what it cost to learn.
- Anything the linker produces is checked rather than trusted:
  `xtask/src/elf.rs` reads the linked kernel back after every build and
  requires the properties `kernel/linker.ld` is written to produce (D-40).
  A linker script fails loudly when malformed and quietly when incomplete.
- A `libs/` crate goes in `default-members` so a bare `cargo test` covers it,
  and in `HOST_PACKAGES` in `xtask/src/paths.rs` so `cargo xtask ci` does too.
- `xtask` logic is unit-tested directly, in-module. Anything touching the
  filesystem is driven through a `Cursor` rather than a real file.
- Constructors that read the environment are split from pure ones —
  `Paths::new` takes everything explicitly, `Paths::discover` reads env and
  delegates. `std::env::set_var` is `unsafe` in edition 2024 and unsound across
  test threads, so tests never touch the environment.
- On-machine checks live in `kernel/src/selftest.rs`, compiled only under the
  `selftest` feature and run by `cargo xtask test-boot`, which boots the kernel
  headless and reads its verdict from QEMU's exit code (D-28). Keep each check
  small, named as the property it asserts, and free of side effects on the
  running kernel — they execute against a live machine, not a fixture.
- A check that cannot fail is worth nothing. When adding one, break it once and
  confirm the harness goes red before trusting it green.
- Hardware behaviour is *also* verified by eye: `cargo xtask run` for the loop,
  a VMware boot as every phase's definition of done (D-18). The self-test
  catches regressions; the eye catches things nobody thought to assert.

## Building

`cargo xtask ci` is the gate, and the only one (D-26). The mechanics —
why `cargo build --workspace` cannot work, the `k*` aliases that reach the
kernel crates, and why `RUSTFLAGS` must never be set here — are in the README's
Development section rather than repeated.

Two conventions that belong with the code rather than the README:

- Set `CARGO_TARGET_DIR` if you want artifacts elsewhere. `build.target-dir` in
  a cargo config is **not** honoured by xtask, which would have to parse
  `cargo metadata` to see it.
- Anything host-specific stays out of `rust-toolchain.toml`. Every host reads
  that file, and rustup resolves `components` against whichever one it is
  running on — naming a Windows-only component breaks every other host (D-24).

## Sources

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) — the
  default standard; `C-XXX` above are its
  [checklist](https://rust-lang.github.io/api-guidelines/checklist.html) items.
- [Rust for Linux coding guidelines](https://docs.kernel.org/rust/coding-guidelines.html)
  — the `// SAFETY:` / `# Safety` distinction, the doc-comment first-sentence
  rule, and the ban on repeating a namespace inside an identifier.
- [Asterinas OSTD](https://asterinas.github.io/book/ostd/) — the privileged
  framework this project's `wost` follows: a ~10k-line minimal core whose safe
  APIs cover memory, tasks, user space, interrupts, timers, drivers, boot, and
  synchronisation. Its stated posture on soundness — argue the high-level design
  and rely on review rather than waiting for formal verification — is the one
  taken here.
- [Oxide's Hubris](https://github.com/oxidecomputer/hubris) — precedent for
  `xtask` over bare cargo in a multi-target embedded build, and for its driver
  naming (`SYSTEM-DEVICE` libraries, `SYSTEM-DEVICE-server` binaries) worth
  copying when `drv/` crates arrive.
- [`rust-osdev/volatile`](https://github.com/rust-osdev/volatile) — the design
  argument behind `MmioRegion`: declare a region safe to access with a specific
  set of operations and give it a type, rather than casting raw pointers at the
  point of use.
- [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/)
  (Microsoft) — the closed taxonomy of valid reasons to write `unsafe` in the
  crate-boundaries section above, and the recommendation to enable
  `unsafe_op_in_unsafe_fn`, which this workspace already denies (D-41).
- [Rustonomicon — Working with Unsafe](https://doc.rust-lang.org/nomicon/working-with-unsafe.html)
  — the sound-wrapper principle every `wost` export is held to. The
  [Unsafe Code Guidelines Reference](https://rust-lang.github.io/unsafe-code-guidelines/)
  is deliberately *not* listed as an authority: it is advisory and partly
  historical by its own description.
