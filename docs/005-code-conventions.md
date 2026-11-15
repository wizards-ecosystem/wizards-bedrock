# 005 - Code conventions

How this repository is written. Adopted 2026-08-12 alongside the framekernel
crate split (D-21); the lint policy is in the workspace lint table in
`Cargo.toml` (D-22, D-451).

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
| `wost/` | **yes, and nowhere else does** | Privileged framework: boot handoff, port I/O, MMIO, CPU control, locking. The kernel's memory-safety TCB. |
| `kernel/` | no (`#![deny(unsafe_code)]`) | The kernel binary: drivers, console, logging, panic reporting, and scheduler. |
| `libs/` | no (`#![forbid(unsafe_code)]`) | Pure, hardware-free logic, host-testable. |
| `xtask/` | no | Host build tooling. Not part of the kernel. |

**What may go in `wost`:** an operation that cannot be expressed safely: a
raw instruction, a raw pointer, a hardware register. Nothing else. If a
function touches no hardware and holds no raw pointer, it does not belong in
`wost`; it belongs in `kernel/` or, better, in a `libs/` crate where it can be
unit-tested on the host.

That test is a judgement, so it is backed by a closed list. **The only valid
reasons to write `unsafe` are** (Pragmatic Rust Guidelines, D-41):

1. **a novel abstraction** the language cannot express: a frame type, an
   allocator, a page table;
2. **performance**, where a checked form exists and measurably costs too much;
3. **FFI or the platform**: an instruction, a register, a bootloader ABI.

Anything else is a defect, not a trade-off. Reason 2 needs a number attached
before it counts, and so far nothing in this kernel has used it.

**The standard every `wost` export is held to** is the Rustonomicon's
sound-wrapper principle: *safe code must not be able to cause undefined
behaviour through this abstraction, however it is called.* The question for a
new export, before its `# Safety` section is written, is not whether the
unsafe block is correct but whether any caller, including a wrong one, can
reach UB.

The Unsafe Code Guidelines Reference is **not** an authority here. It
describes itself as largely historical and advisory, and documents open
questions alongside settled rules; cite the Nomicon or the reference, not it.

**No speculative API.** Do not add an operation because a subsystem will
plausibly want it later. This matters most in `wost`, where an unused
`pub unsafe fn` carries a safety contract nobody can check. Write it alongside
its first caller, when the obligation is real and testable. The exception is
completing a primitive that is incoherent in halves: `PortValue` covers 8, 16,
and 32 bits because a sealed trait with one impl is not an abstraction.

**What `wost` owes its callers:** every abstraction it exports must make the
unsafe operation underneath it *checkable*, not merely *wrapped*.
`MmioRegion` bounds- and alignment-checks. `port::claim` records ownership in a
registry so exclusivity is verified rather than promised, over the span an
access *reaches*, not the port it names (D-39). A spin lock's recovery
operation and its proof value (`PanicToken`) are crate-private, and only the
non-returning panic boundary can reach them, so a safe caller cannot manufacture
recovery authority or resume after violating a live guard. Following Asterinas
(`002-architecture.md` pillar 5): these abstractions are designed in from day
one, never retrofitted.

A bound stated in prose is not a bound. Where the checkable form needs
arithmetic, the arithmetic goes to a `libs/` crate and `wost` keeps only the
lock and the instruction. `port_space::PortSpace` under
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

- One concept per file. A module with children is `name.rs` beside `name/`;
  there is no `mod.rs`.
- Architecture-specific code lives under `arch/<arch>/`, reached through
  `arch::current` so callers never name a concrete architecture.
- A module that names an external protocol or vendor crate should be the *only*
  one that does. `wost::boot::limine_glue` is the whole of the bootloader
  dependency; swapping bootloaders touches one file.
- Items are `pub(crate)` rather than `pub` unless another crate calls them, and
  `unreachable_pub` enforces it.

## Naming

- Conventional Rust throughout. The wizard theme lives in product-facing
  strings (banners, app names, shell personality) and never in identifiers
  (D-19).
- Spell things out: `registers`, `intensity`, `framebuffer`. Established
  systems jargon stays short: `uart`, `mmio`, `lba`, `bpp`, `esp`.
- Constants replace literals. A bare `0xC7` in a driver is a defect; a
  `fifo_control::ENABLE | fifo_control::CLEAR_RECEIVE | ...` is the same byte
  and says what it means.
- **Never repeat a namespace inside an identifier** (Rust for Linux). The module
  path already carries it: `register::LINE_STATUS`, not `UART_REGISTER_LINE_STATUS`.
  Group related constants in a small module rather than prefixing names.
- Casing per RFC 430, and `as_` / `to_` / `into_` used with their standard
  meanings (C-CASE, C-CONV).
- Consistent word order across an API (C-WORD-ORDER): verb then noun, and the
  same verb for the same operation: `fill_rows` / `copy_rows`, `write_u32` /
  `read_u32`.
- **Arguments convey meaning through types, not `bool`** (C-CUSTOM-TYPE).
  `fmt(paths, true)` says nothing; `fmt(paths, FmtMode::Check)` says everything.
  This applies hardest to two adjacent parameters of the same type, hence
  `Palette` rather than a `(background, foreground)` pair the compiler would
  happily let you swap.
- Imports in three blocks, blank-line separated: `core`/`std`, external crates,
  then `crate`. This is convention rather than configuration: rustfmt's
  `group_imports` is nightly-only and stable `cargo fmt` ignores it silently.
  Rust for Linux's one-item-per-line import layout is **not** adopted: it
  contradicts the rustfmt defaults this project formats with.

## Comments

**Few comments. A comment explains WHY, never WHAT.** If a comment restates
the code, delete one of them, usually the comment. A comment never cites a
decision (D-451): a decision file names the code it governs, so the reference
runs from the decision to the code and never back.

Exactly two kinds of prose are mandatory, and both are enforced:

- `// SAFETY:` before every `unsafe` block and `unsafe impl`, saying which
  precondition holds and why (`clippy::undocumented_unsafe_blocks`). One unsafe
  operation per block, so each comment covers exactly one thing
  (`clippy::multiple_unsafe_ops_per_block`).
- `# Safety` on every `pub unsafe fn`, stating the caller's obligation
  (`clippy::missing_safety_doc`).

The distinction is easy to blur (Rust for Linux): a `# Safety` section states
the *precondition a caller must meet*; a `// SAFETY:` comment states *why this
particular call meets it*. One is a contract, the other is evidence.

Beyond that: `//!` on a module whose purpose its name does not say, with what
it assumes; `///` on a public item whose name does not say what it does, whose
first line is a single sentence; `# Panics` on anything that can panic
(C-FAILURE). `missing_docs` and `missing_docs_in_private_items` are not
enabled, because they manufacture noise on every helper.

C-EXAMPLE is adopted **only in `libs/`**, because that is where doctests can
run (D-41). `wost` and `kernel` are `no_std` and cross-compiled, so their
doctests cannot execute and an unrunnable example rots unnoticed. A `libs/`
crate carries `#![cfg_attr(not(test), no_std)]` so `cargo test` builds it for
the host, where its doctests compile and run under the bare `cargo test` that
`default-members` scopes. Put an example on any item whose encoding an example
states better than prose; `libs/x86-paging`'s entry bits are the worked case.
No example is required per crate or per item.

Rustdoc conventions:

- The first line is the module listing and the search result, so it is one
  sentence, ends in a period, and stays under about fifteen words.
- Sections appear in the conventional order (summary, body, `# Examples`,
  `# Errors`, `# Panics`, `# Safety`), the order every experienced reader scans for.
- `#[doc(hidden)]` on items that are `pub` only for a technical reason.
  `boot::__macro` is the case: `entry!` needs those names in scope in the
  calling crate, and nobody should read them as API.

The comments worth writing record a decision the code cannot show: why the
divisor is 1, why the entry glue expands in the binary crate, why a non-zero
QEMU exit is not an error.

## Documentation

How the docs in `docs/` and the root documents are written (D-451).

- Plain declarative sentences. The present tense says what is true now; the
  past tense with a date says what was observed. The first sentence of a
  section says what it decides or describes.
- One idea per sentence, usually 12 to 22 words; split anything over 35.
- Name the thing: a file path, a function, a decision, a row. Define a term
  where it first matters, once per doc.
- Bold only for a labelled paragraph lead (`**Decision.**`) and for a normative
  word a reader must not miss, not for emphasis.
- No em or en dashes. Use a comma, a colon, parentheses, "to" for a range, or
  two sentences.
- No intensifiers or self-certification ("honest", "load-bearing", "the whole
  point"), no antithesis used as rhetoric, no narration of how a text was made.
  Keep the fact and drop the frame.
- A dated claim keeps its date and its tense; never refresh one by changing the
  date. A rule an amending decision changed stays in the past tense where it is
  recounted.
- A doc states a count only where a check keeps it true; otherwise it points
  at the place that does (D-124).
- Decisions are files in `docs/decisions/` indexed by `004-decision-log.md`,
  and a decision body is never edited after it lands; an amendment is a new
  decision.
- Public text names no external runtime or its author.

## Error handling

**Kernel and `wost`:**

- No `unwrap`, no `expect`, no panicking indexing on any path that runs at
  runtime (`clippy::unwrap_used` and `clippy::expect_used` are denied in `wost`
  and in the shipped kernel). Return `Result` where a caller can do something,
  or an `Option` where absence is ordinary.
- `assert!` is for invariants that are genuinely unrecoverable, and its message
  says what was violated: a rejected bootloader base revision, a register
  index outside its block. `debug_assert` is refused (`disallowed_macros`), so
  no production invariant is debug-only.
- Make total what can be made total. Glyph lookup falls back rather than
  unwrapping; a pixel outside the visible area is clipped rather than checked
  by the caller.
- Error types are enums, never strings, and they are **well-behaved**
  (C-GOOD-ERR): `Debug`, `Display`, and `core::error::Error`, which is available
  in `core` since Rust 1.81 and needs no crate. `Display` writes a lowercase
  clause with no trailing period, so it reads correctly inside a sentence:
  `"no serial log: no controller answered the loopback probe"`.
- Public error enums in `libs/` and `wost` carry `#[non_exhaustive]`, so a new
  variant is not a breaking change and every `match` on one is written to
  survive growth.
- **Types implement the common traits eagerly** (C-COMMON-TRAITS, C-DEBUG):
  `Debug` on everything public, enforced by `missing_debug_implementations`, plus
  `Clone`/`Copy`/`PartialEq`/`Eq` on plain data. `Debug` matters more here than
  in most Rust: serial output is the only debugger there is. A `Debug` impl must
  never block. `SpinLock`'s uses `try_lock` and prints `<locked>` rather than
  spinning, because the context most likely to print one is the panic handler.

**xtask:** `anyhow` with `.context(...)` on every fallible call, phrased so
the message reads as "while doing X" when chained. No `unwrap`.

## Tests

The suite is small and every check earns its place (D-454; `AGENTS.md` lists
its six tiers). Where a check goes is a property of the thing being checked:

| The thing under test | Where it goes | How it runs |
|---|---|---|
| Arithmetic, bit layout, parsing, with no hardware | a `libs/` crate | `cargo test` |
| Anything needing a real machine | `kernel/src/selftest.rs` | `cargo xtask test-boot` |
| Tooling a user runs | a host test in `xtask/` | `cargo test` |

A change adds at most one focused test, and only for a real bug. Do not test
what the code merely restates: a constant, a table, a layout, a getter, a type,
a whole-output snapshot or a mock call. Do not read source text in a test. A
rule about source is a lint, a `deny` or `forbid` attribute or a type:
`unsafe_code` is `forbid` in every `libs/` crate and `deny` in `kernel`, the
clippy unsafe lints enforce `// SAFETY:` and one operation per block, and
`kernel` and `wost` refuse `debug_assert` so no production invariant is
debug-only. A flaky or slow check is deleted, not fixed.

`cargo test -p wizard-kernel` will never work: a `#![no_std] #![no_main]`
binary cannot host the `test` harness, and in-QEMU `#[test_case]` runners need
nightly's `custom_test_frameworks`, which D-01 rules out. So:

- Pure logic gets extracted to a `libs/` crate carrying
  `#![cfg_attr(not(test), no_std)]`, where `cargo test` runs it on the host.
  `libs/pixel` is the worked example: colour, channel packing, and framebuffer
  offset arithmetic, the code that carried a live out-of-bounds bug while it
  was buried in the kernel. `libs/port-space` shows why: the I/O port
  reservation bitmap sat inside `wost`, where nothing could test it, and it
  recorded one port per claim however wide the access it authorised (D-39).
- A cargo feature is a different crate graph, and a lint that fires in one need
  not fire in the other. `cargo xtask ci` covers the host packages plus the
  shipped, self-test and `refuse-pci` kernel graphs; adding a build-only feature
  means adding a graph to `xtask/src/gate.rs`, whose registry also drives
  the help text (D-212).
- Anything the linker produces is checked rather than trusted:
  `xtask/src/elf.rs` reads the linked kernel back after every build and
  requires the properties `kernel/linker.ld` is written to produce (D-40).
  A linker script fails loudly when malformed and quietly when incomplete.
- A `libs/` crate is a workspace member and goes in `default-members`, so a
  bare `cargo test` covers it; the gate's host pass covers every member except
  `wost` and the kernel.
- Constructors that read the environment are split from pure ones:
  `Paths::new` takes everything explicitly, `Paths::discover` reads env and
  delegates. `std::env::set_var` is `unsafe` in edition 2024 and unsound across
  test threads, so tests never touch the environment.
- On-machine checks live in `kernel/src/selftest.rs` and its children, compiled
  only under the `selftest` feature and run by `cargo xtask test-boot`, which
  boots the kernel headless and reads its verdict from QEMU's exit code. Keep
  each check small, named as the property it asserts, and free of side effects
  on the running kernel.
- A check that cannot fail is worth **less than no check**, because it is also
  a claim. **Write the check from the claim, not from the implementation.**
  Restating what the code does always passes and reads exactly like a check:
  `assert_eq!(GENERATION_BITS, 32 - INDEX_BITS - RESERVED_BITS)` is a
  definition (D-124).
- **Choose inputs where the answers differ.** A realistic test value can be
  worse than a contrived one: a real `IA32_APIC_BASE` value that sets both the
  bootstrap and enable bits was satisfied by the two constants swapped (D-142),
  and a framebuffer whose pitch equals its visible row cannot tell `pitch` from
  `width * bpp` (D-155). Make the values pairwise distinct, so no field or
  constant can stand in for another (D-136).
- **A check must reach the code it is about.** Set-up chosen to make a failure
  easy to trigger can make it impossible to reach: a full table refuses early,
  a closed channel refuses early, an unmapped page faults early. If the refusal
  under test is the second thing that could go wrong, the input has to get past
  the first (D-165).
- **A check that quantifies must read its domain.** If the name begins "every",
  "all" or "no", the body enumerates the domain rather than naming its members;
  a list restated beside its source passes on the day the two disagree (D-157).
- **Measure what a tool does; do not reason about it.** `#[expect(lint)]` fails
  the build when the suppression stops being needed, except inside a macro
  expansion, where it never fires and so reads as checked while being as
  unfalsifiable as the `allow` it replaced (D-131). A break that cannot run
  produces the appearance of verification: absence of a pass is not a failure,
  and only a red check is evidence (D-175).
- Hardware behaviour is *also* verified by eye: `cargo xtask run` for the loop,
  a VMware boot as every phase's definition of done (D-18). The self-test
  catches regressions; the eye catches things nobody thought to assert.
- `cargo mutants` over changed library code runs on the laptop on demand;
  nothing requires or records it (D-454).

## Building

`cargo xtask ci` is the gate, and the only one (D-26). The mechanics (why
`cargo build --workspace` cannot work, the `k*` aliases that reach the
kernel crates, and why `RUSTFLAGS` must never be set here) are in the README's
"Building & running" section rather than repeated.

Two conventions that belong with the code rather than the README:

- Set `CARGO_TARGET_DIR` if you want artifacts elsewhere. `build.target-dir` in
  a cargo config is **not** honoured by xtask, which would have to parse
  `cargo metadata` to see it.
- Anything host-specific stays out of `rust-toolchain.toml`. Every host reads
  that file, and rustup resolves `components` against whichever one it is
  running on. Naming a Windows-only component breaks every other host (D-24).

## Sources

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/): the
  default standard; `C-XXX` above are its
  [checklist](https://rust-lang.github.io/api-guidelines/checklist.html) items.
- [Rust for Linux coding guidelines](https://docs.kernel.org/rust/coding-guidelines.html):
  the `// SAFETY:` / `# Safety` distinction, the doc-comment first-sentence
  rule, and the ban on repeating a namespace inside an identifier.
- [Asterinas OSTD](https://asterinas.github.io/book/ostd/): the privileged
  framework this project's `wost` follows: a ~10k-line minimal core whose safe
  APIs cover memory, tasks, user space, interrupts, timers, drivers, boot, and
  synchronisation. Its stated posture on soundness (argue the high-level design
  and rely on review rather than waiting for formal verification) is the one
  taken here.
- [Oxide's Hubris](https://github.com/oxidecomputer/hubris): precedent for
  `xtask` over bare cargo in a multi-target embedded build, and for its driver
  naming (`SYSTEM-DEVICE` libraries, `SYSTEM-DEVICE-server` binaries) worth
  copying when `drv/` crates arrive.
- [`rust-osdev/volatile`](https://github.com/rust-osdev/volatile): the design
  argument behind `MmioRegion`: declare a region safe to access with a specific
  set of operations and give it a type, rather than casting raw pointers at the
  point of use.
- [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/)
  (Microsoft): the closed taxonomy of valid reasons to write `unsafe` in the
  crate-boundaries section above, and the recommendation to enable
  `unsafe_op_in_unsafe_fn`, which this workspace already denies (D-41).
- [Rustonomicon, Working with Unsafe](https://doc.rust-lang.org/nomicon/working-with-unsafe.html):
  the sound-wrapper principle every `wost` export is held to. The
  [Unsafe Code Guidelines Reference](https://rust-lang.github.io/unsafe-code-guidelines/)
  is deliberately *not* listed as an authority: it is advisory and partly
  historical by its own description.
