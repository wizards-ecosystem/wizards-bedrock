# 005 - Code conventions

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
| `wost/` | **yes, and nowhere else does** | Privileged framework: boot handoff, port I/O, MMIO, CPU control, locking. The kernel's memory-safety TCB. |
| `kernel/` | no (`#![deny(unsafe_code)]`) | The kernel binary: drivers, console, logging, panic reporting, scheduler, and eventually VFS. |
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
`pub unsafe fn` carries a safety contract nobody can check. `enable_interrupts`
was written and then removed for exactly this reason: its contract has to talk
about an IDT, and until one exists the contract is a guess. Write it alongside
its first caller, when the obligation is real and testable. The exception is
completing a primitive that is incoherent in halves: `PortValue` covers 8, 16,
and 32 bits because a sealed trait with one impl is not an abstraction.

**What `wost` owes its callers:** every abstraction it exports must make the
unsafe operation underneath it *checkable*, not merely *wrapped*.
`MmioRegion` bounds- and alignment-checks. `port::claim` records ownership in a
registry so exclusivity is verified rather than promised, over the span an
access *reaches*, not the port it names, which is the distinction Phase 2 got
wrong and D-39 fixed. A spin lock's recovery operation and its proof value are
crate-private, and only the non-returning panic boundary can reach them. A
safe caller cannot manufacture recovery authority or resume after violating a
live guard. Following Asterinas
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

- One concept per file; no `mod.rs` unless the module has children.
- Architecture-specific code lives under `arch/<arch>/`, reached through
  `arch::current` so callers never name a concrete architecture.
- A module that names an external protocol or vendor crate should be the *only*
  one that does. `wost::boot::limine_glue` is the whole of the bootloader
  dependency; swapping bootloaders touches one file.
- In a binary crate, items are `pub(crate)` rather than `pub`, and
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
  Group related constants in a small private module rather than prefixing names.
- Casing per RFC 430, and `as_` / `to_` / `into_` used with their standard
  meanings (C-CASE, C-CONV).
- Consistent word order across an API (C-WORD-ORDER): verb then noun, and the
  same verb for the same operation: `fill_rows` / `copy_rows`, `write_u32` /
  `read_u32`. The example here used to be `write_pixel` / `read_pixel`; there
  is no `read_pixel` and there is not meant to be, because framebuffer memory
  is uncached and reading it is what the back buffer exists to avoid.
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
the code, delete one of them, usually the comment.

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
(C-FAILURE). `missing_docs_in_private_items` is deliberately **not** enabled,
because it manufactures noise on every helper.

C-EXAMPLE is adopted **only in `libs/`**, and the split is a fact about where
doctests can run rather than a preference (D-41). The floor is one runnable
example per `libs/` crate (on the module doc or its central item), and all
current crates carry one; more go on any item whose encoding an example states
better than prose. Not "an
example on every item", which the rule used to say and no crate has ever
done. `wost` and `kernel` are `no_std` and cross-compiled, so their doctests
cannot execute and an unrunnable example rots unnoticed. A `libs/` crate
carries `#![cfg_attr(not(test), no_std)]` precisely so `cargo test` builds it
for the host, where its doctests *do* compile and run, under the bare
`cargo test` that `default-members` already scopes. There an example is an
executable check, and for an encoding API it is the clearest statement of the
encoding there is: `libs/x86-paging`'s entry bits are the worked case.

Two rustdoc conventions that go with it:

- The first line is the module listing and the search result, so it is one
  sentence, ends in a period, and stays under about fifteen words.
- Sections appear in the conventional order (summary, body, `# Examples`,
  `# Errors`, `# Panics`, `# Safety`) because that is the order every
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
  says what was violated: a rejected bootloader base revision, a register
  index outside its block.
- Make total what can be made total. Glyph lookup falls back rather than
  unwrapping; a pixel outside the visible area is clipped rather than checked
  by the caller.
- Error types are enums, never strings, and they are **well-behaved**
  (C-GOOD-ERR): `Debug`, `Display`, and `core::error::Error`, which is available
  in `core` since Rust 1.81 and needs no crate. `Display` writes a lowercase
  clause with no trailing period, so it reads correctly inside a sentence:
  `"no serial log: no controller answered the loopback probe"`.
- Error enums carry `#[non_exhaustive]`. A new variant is then not a breaking
  change, and every `match` on one is written to survive growth.
- **Types implement the common traits eagerly** (C-COMMON-TRAITS, C-DEBUG):
  `Debug` on everything public, enforced by `missing_debug_implementations`, plus
  `Clone`/`Copy`/`PartialEq`/`Eq` on plain data. `Debug` matters more here than
  in most Rust: serial output is the only debugger there is. A `Debug` impl must
  never block. `SpinLock`'s uses `try_lock` and prints `<locked>` rather than
  spinning, because the context most likely to print one is the panic handler.

**xtask:** `anyhow` with `.context(...)` on **every** fallible call, phrased so
the message reads as "while doing X" when chained. No `unwrap`.

## Tests

**Every change lands with the check that proves it.** There are three places
a check can go, and which one is a property of the thing being checked, not a
preference:

| The thing under test | Where it goes | How it runs |
|---|---|---|
| Arithmetic, bit layout, parsing, with no hardware | a `libs/` crate | `cargo test` |
| Anything needing a real machine | `kernel/src/selftest.rs` | `cargo xtask test-boot` |
| **What this repository says about itself** | a host test in `xtask/` | `cargo test` |

The third row is the newest and the least obvious. A document that names a
crate, a constant that states a count, a comment that claims a privilege: each
is an assertion about the tree, and each was found wrong during the Phase 0-6
sweep. So they are checked like any other: `xtask/src/decisions.rs` requires
every `D-NNN` cited anywhere to name an entry the log defines and the log to
have no gaps; `xtask/src/prose.rs` requires every `libs/` crate the documents
name to exist or be declared absent with a reason, and the README's marked CI
table to match the typed registry execution reads; `xtask/src/unsafety.rs` pins `wost`'s
unsafe line count rather than only capping it; `xtask/src/qemu.rs` compares the
C header against the kernel's own constants, discovers every active sysdep,
checks its BODY evidence, and checks its two-axis source marker against the exact
register in `userland/README.md`. Prose that cannot be
checked should not state a number: point at the place that can (D-124).

If a check fits none of the three, that is usually a sign the logic should be
split so that the interesting half fits the first row. `libs/pixel` exists because the
framebuffer's arithmetic was worth testing and its hardware access was not.

`cargo test -p wizard-kernel` will never work: a `#![no_std] #![no_main]`
binary cannot host the `test` harness, and in-QEMU `#[test_case]` runners need
nightly's `custom_test_frameworks`, which D-01 rules out. So:

- Pure logic gets extracted to `libs/` crates carrying
  `#![cfg_attr(not(test), no_std)]`, where `cargo test` runs it on the host.
  `libs/pixel` is the worked example: colour, channel packing, and framebuffer
  offset arithmetic: the code that carried a live out-of-bounds bug while it
  was buried in the kernel. `libs/x86-tables` is the second: GDT, IDT and TSS
  encoding, where the host test is the only thing that can check a bit position
  against the manual. `libs/x86-apic` and `libs/x86-pit` follow on the same
  grounds, and `libs/ps2-keys` for the scancode state machine.
  `libs/port-space` is a particularly clear case of *why*: the I/O port
  reservation bitmap sat inside `wost`, where nothing could test it, and it
  spent Phase 2 recording one port per claim however wide the access it
  authorised (D-39). Page-table bit packing has since become
  `libs/x86-paging`, ELF parsing `libs/elf64`, ring index arithmetic
  `libs/ring-proto`, and handle encoding `libs/handle-space`; add crates on the
  same pattern for the WizFS on-disk format and its crash-injection harness as
  those arrive.
- **The gate runs over a configuration matrix, not a codebase.** A cargo
  feature is a different crate graph, and a lint that fires in one need not
  fire in the other: `cargo xtask ci` covers the host packages plus shipped,
  self-test and `refuse-pci` kernel graphs. Adding a build-only feature means
  adding a graph to `xtask/src/gate.rs`; D-212 makes the same registry drive
  execution, help and README, while D-22 records what missing the shipped graph
  once cost.
- **Public foundation APIs need an exact compiler-resolved caller.** `cargo
  xtask api-audit` inventories public item declarations and enum variants in
  `libs/` and `wost`; re-exports and documentation are not callers. Structured
  compiler evidence must bind the exact Cargo graph and source span, and every
  route the audit cannot classify fails closed. Fields remain outside the claim
  because stable rustc omits some live destructuring diagnostics. The
  [development guide](../README.md#development) records the operational model;
  D-213 and D-275 record its decisions and trust limits.
- Anything the linker produces is checked rather than trusted:
  `xtask/src/elf.rs` reads the linked kernel back after every build and
  requires the properties `kernel/linker.ld` is written to produce (D-40).
  A linker script fails loudly when malformed and quietly when incomplete.
- A `libs/` crate goes in `default-members` so a bare `cargo test` covers it,
  and in `HOST_PACKAGES` in `xtask/src/paths.rs` so `cargo xtask ci` does too.
- `xtask` logic is unit-tested directly, in-module. Anything touching the
  filesystem is driven through a `Cursor` rather than a real file.
- Constructors that read the environment are split from pure ones:
  `Paths::new` takes everything explicitly, `Paths::discover` reads env and
  delegates. `std::env::set_var` is `unsafe` in edition 2024 and unsound across
  test threads, so tests never touch the environment.
- On-machine checks live in `kernel/src/selftest.rs`, compiled only under the
  `selftest` feature and run by `cargo xtask test-boot`, which boots the kernel
  headless and reads its verdict from QEMU's exit code (D-28). Keep each check
  small, named as the property it asserts, and free of side effects on the
  running kernel. They execute against a live machine rather than a fixture.
- A check that cannot fail is worth **less than no check**, because it is also
  a claim. When adding one, break it once and confirm the harness goes red
  before trusting it green (D-28).

  Two things the sweeps since have added to that rule, both learned the hard
  way. **Breaking it by hand happens once**, at the moment the check is
  written, and never again, so a check that *stops* being able to fail,
  because the code around it moved, is invisible from then on. `cargo xtask
  mutate` is that rule made repeatable: each entry is a real defect applied to
  the tree, and a boot must go red (D-119). And **write the check from the
  claim, not from the implementation**. Restating what the code does always
  passes and reads exactly like a check: `assert_eq!(GENERATION_BITS, 32 -
  INDEX_BITS - RESERVED_BITS)` is a definition, and a frame-allocator check
  that asserts the address, the count and the tally has checked the
  allocator's *description* rather than its frames (D-124).
- **Choose inputs where the answers differ.** A realistic test value can be
  worse than a contrived one, and this has now cost three checks.
  `TYPICAL = 0xFEE0_0900` is a real `IA32_APIC_BASE` value that sets both the
  bootstrap and enable bits, so every test using it was satisfied by the two
  constants swapped (D-142). A framebuffer whose pitch equals its visible row
  is a real configuration in which `pitch` and `width * bpp` are
  interchangeable, so computing the span from the wrong one survives (D-155).
  A clock frequency whose ratio lands nowhere near a halfway point is a real
  clock that proves nothing about rounding.

  Two rules follow, and they are the same rule from opposite ends: **make the
  values pairwise distinct**, so no field or constant can stand in for another
  (D-136); and **check the claim, not the trick**: "the multiplier is the
  nearest integer to the ratio" is testable against both neighbours without
  ever mentioning the `+ denominator / 2` that achieves it.

- **A break is only red if the thing that went red is a check.** Absence of a
  pass is not a failure: a build error, a harness crash, a timeout and a red
  check all produce no verdict, and only one of them is evidence. A break
  script that greps for the absence of success is measuring the wrong thing,
  and a break that cannot run produces the *appearance* of having verified
  something, which is what does the damage (D-175).

- **A check must reach the code it is about.** Set-up chosen to make a failure
  easy to trigger can make it impossible to *reach*: a full table refuses
  early, a closed channel refuses early, an unmapped page faults early. If the
  refusal under test is the second thing that could go wrong, the input has to
  get past the first. A check for "a promised handle slot is given back on
  every refusal" used a table with no room, so every refusal happened at the
  reservation and no promise was ever made. It passed with the destructor that
  returns promises deleted, and nothing about the assertion looked wrong,
  because the assertion was not wrong (D-165).

- **A check that quantifies must read its domain.** If the name begins
  "every", "all" or "no", the body enumerates the domain rather than naming
  its members. A list restated beside its source is a second copy with a
  test-shaped wrapper, and it passes on the day the two disagree, which is the
  failure it was written to catch. `vendor.rs` is the shape: it walks the tree
  for `CHECKSUMS` files, cross-checks both directions and asserts the counts
  match. The two checks that were wrong were the two that had a list (D-157).

- **Measure what a tool does; do not reason about it.** This is the one rule
  here that was learned twice in a day, in opposite directions, and it is
  narrower and more useful than "check your assumptions".

  `#[expect(lint)]` fails the build when the suppression stops being needed,
  except inside a macro expansion, where `unfulfilled_lint_expectations` never
  fires. Converting a macro's attributes therefore produces something that
  *reads as checked* and is exactly as unfalsifiable as the `allow` it
  replaced, which is worse than leaving it. Found by adding an
  `#[expect(non_snake_case)]` to a `SCREAMING_CASE` static and watching nothing
  happen (D-131).

  The mutation sweep's `strip_suffix(" FAILED")` looked stricter than the
  verdict check it replaced. Serial lines carry a trailing `
`, so it matched
  nothing, and the first strict run reported **eighteen survivors on boots that
  had each reddened exactly the right check**: a stricter-looking tool that
  was strictly worse, reporting failure for defects that were caught (D-134).

  Both were ten minutes to measure and would have been a phase to find. The
  pattern is not that premises need checking in general: it is that **a tool is
  a thing we describe to ourselves in prose and then trust the prose**, so the
  gap between what it does and what we say it does closes only when something
  runs.

- Hardware behaviour is *also* verified by eye: `cargo xtask run` for the loop,
  a VMware boot as every phase's definition of done (D-18). The self-test
  catches regressions; the eye catches things nobody thought to assert.

## Building

`cargo xtask ci` is the gate, and the only one (D-26). The mechanics (why
`cargo build --workspace` cannot work, the `k*` aliases that reach the
kernel crates, and why `RUSTFLAGS` must never be set here) are in the README's
Development section rather than repeated.

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
