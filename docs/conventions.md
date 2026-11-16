# Conventions

How code and docs are written in this repository. The default standard is the
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/), with the
[Rust for Linux coding guidelines](https://docs.kernel.org/rust/coding-guidelines.html) taking
precedence where they are more specific about kernel work. This file records only what those
leave open and what this project decides differently. `C-XXX` names an API Guidelines checklist
item.

The compiler enforces what it can, and the rest is written down here. Where a convention can be a
lint, it is one: the `[workspace.lints]` tables in `Cargo.toml` govern every crate, `xtask`
included, and the gate runs clippy with warnings as errors. The table carries the unsafe rules,
the panic and debug-leftover bans and the house style (`Self`, `let else`, inline format
arguments, assert messages, no wildcard imports).

## Crate boundaries

| Crate | May write `unsafe` | Purpose |
|---|---|---|
| `wost/` | yes | the privileged framework and the kernel's memory-safety base |
| `witness/rt/` | yes, and no crate but `wost` | the ring 3 runtime: entry, call gate, ring access and probes |
| `kernel/` | no (`#![deny(unsafe_code)]`) | the kernel binary |
| `witness/ring3/` | no (`#![deny(unsafe_code)]`) | the ring 3 witness program |
| `libs/` | no (`#![forbid(unsafe_code)]`) | pure, hardware-free logic, tested on the host |
| `xtask/` | no (`#![forbid(unsafe_code)]`) | host build tooling, not part of the kernel |

What may go in `wost` is an operation that cannot be expressed safely: a raw instruction, a raw
pointer, a hardware register. A function that touches no hardware and holds no raw pointer belongs
in `kernel/` or, better, in a `libs/` crate where the host can test it.

That test is a judgement, so it is backed by a closed list. The only valid reasons to write
`unsafe` are:

1. a novel abstraction the language cannot express: a frame type, an allocator, a page table;
2. performance, where a checked form exists and measurably costs too much;
3. FFI or the platform: an instruction, a register, a bootloader ABI.

Anything else is a defect. Reason 2 needs a measurement before it counts, and nothing in this
kernel uses it yet.

Every `wost` export is held to the Rustonomicon's sound-wrapper principle: safe code must not be
able to cause undefined behaviour through it, however it is called. The question for a new export
is whether any caller, including a wrong one, can reach undefined behaviour, not whether the
`unsafe` block inside is correct. Cite the Rustonomicon or the Rust Reference for soundness, not
the Unsafe Code Guidelines Reference, which describes itself as advisory.

Every abstraction `wost` exports makes the operation underneath it checkable, not merely wrapped.
`MmioRegion` checks bounds and alignment. A port claim records ownership in a registry over every
port an access reaches. A spin lock's recovery operation and its `PanicToken` are crate-private,
so only the non-returning panic boundary can use them. Where the checkable form needs arithmetic,
the arithmetic goes to a `libs/` crate and `wost` keeps the lock and the instruction;
`port_space::PortSpace` under `arch::x86_64::port` is the pattern to copy.

No speculative API. An operation is added with its first caller, when its obligation is real and
testable; an unused `pub unsafe fn` carries a contract nobody can check. The exception is a
primitive that is incoherent in halves: `PortValue` covers 8, 16 and 32 bits.

The panic path never blocks on a kernel lock. A fault is not masked by the interrupt flag, so the
code a panic interrupted may hold any lock. `wost` reclaims only its own locks, inside a terminal
boundary that cannot capture a live guard. Kernel reporting uses `try_lock` and omits a channel
whose lock is busy.

Every lock names its level as its first type parameter. A lock is `Leaf` unless code under its
guard takes another lock or runs caller-supplied work; such a lock gets a level in its crate's one
rank list (`kernel/src/rank.rs` in the kernel), chained after every level held when it is taken.

`kernel/` uses `deny`, not `forbid`, because `wost::entry!` expands `#[allow(unsafe_code)]` boot
glue into the binary crate and `forbid` cannot be lifted by any `allow`.

## Modules and layout

- One concept per file. A module with children is `name.rs` beside `name/`; there is no `mod.rs`.
- Architecture-specific code lives under `arch/<arch>/`, reached through `arch::current`, so
  callers never name a concrete architecture.
- A module that names an external protocol or vendor crate is the only one that does.
  `wost::boot` is the whole bootloader dependency.
- Items are `pub(crate)` rather than `pub` unless another crate calls them; `unreachable_pub`
  enforces it.
- Self-test code compiles only under the `selftest` feature and never widens the shipped API.

## Naming

- Conventional Rust throughout. The wizard theme lives in product-facing strings (banners, app
  names) and never in identifiers.
- Spell things out: `registers`, `intensity`, `framebuffer`. Established systems jargon stays
  short: `uart`, `mmio`, `lba`, `bpp`, `esp`.
- Constants replace literals. A bare `0xC7` in a driver is a defect;
  `fifo_control::ENABLE | fifo_control::CLEAR_RECEIVE | ...` is the same byte and says what it
  means.
- Never repeat a namespace inside an identifier: `register::LINE_STATUS`, not
  `UART_REGISTER_LINE_STATUS`. Group related constants in a small module rather than prefixing.
- Casing per RFC 430, and `as_`, `to_` and `into_` with their standard meanings (C-CASE, C-CONV).
- One word order across an API, verb then noun, and one verb per operation: `fill_rows` and
  `copy_rows`, `write_u32` and `read_u32` (C-WORD-ORDER).
- Arguments carry meaning through types, not `bool` (C-CUSTOM-TYPE): `fmt(paths, FmtMode::Check)`,
  not `fmt(paths, true)`. This matters most for two adjacent parameters of one type, which is why
  `Palette` exists rather than a `(background, foreground)` pair.
- Imports in three blank-line-separated blocks: `core` or `std`, external crates, then `crate`.
  Stable `rustfmt` cannot enforce this, so it is a convention. Rust for Linux's one-item-per-line
  imports are not adopted, because they contradict the `rustfmt` defaults.

## Comments

Code carries almost no comments; the design lives in `docs/architecture.md`. What stays:

- `// SAFETY:` before every `unsafe` block and `unsafe impl`, saying which precondition holds and
  why (`clippy::undocumented_unsafe_blocks`). One unsafe operation per block, so each comment
  covers one thing (`clippy::multiple_unsafe_ops_per_block`).
- `# Safety` on every `pub unsafe fn`, stating the caller's obligation
  (`clippy::missing_safety_doc`). A `# Safety` section is the contract a caller must meet; a
  `// SAFETY:` comment is why this call meets it.
- A single-line comment for a non-obvious constraint, a workaround, a lock-order rule or a caller
  obligation the types cannot express.

Nothing else: no other doc comments, no module essays, no comment that restates the code, and no
comment that cites a decision, a doc section or a commit.

## Error handling

Kernel and `wost`:

- No `unwrap`, no `expect` and no panicking indexing on a runtime path; `clippy::unwrap_used` and
  `clippy::expect_used` are denied. Return `Result` where a caller can act, `Option` where absence
  is ordinary.
- Malformed external input (ring entries, images, firmware tables, filesystem and device data)
  ends in a typed error, never a panic.
- `assert!` is for an internal contradiction that breaks ownership or an unsafe precondition, and
  its message says what was violated. `wost` and `kernel` refuse `debug_assert!` through
  `disallowed-macros` in their `clippy.toml`, so no production invariant is debug-only. An operation that can refuse is `#[must_use]`.
- Make total what can be made total. Glyph lookup falls back rather than unwrapping; a pixel
  outside the visible area is clipped.
- Error types are enums, never strings, and are well-behaved (C-GOOD-ERR): `Debug`, `Display` and
  `core::error::Error`. `Display` writes a lowercase clause with no trailing period, so it reads
  inside a sentence: `"no serial log: no controller answered the loopback probe"`.
- Types implement the common traits eagerly (C-COMMON-TRAITS, C-DEBUG): `Debug` on everything
  public (`missing_debug_implementations`), and `Clone`, `Copy`, `PartialEq` and `Eq` on plain
  data. Serial output is the only debugger, and a `Debug` impl never blocks: `SpinLock` prints
  `<locked>` rather than spinning, because the panic handler is the likeliest caller.

`xtask` uses `anyhow` with `.context(...)` on every fallible call, phrased to read as "while doing
X" when chained, and no `unwrap`.

## Tests

The suite is minimal, and each check belongs to one of these tiers.

| Tier | Usually lives in |
|---|---|
| one boot smoke that must settle with its required records and no panic | `cargo xtask boot-check` |
| syscall and ABI behaviour | `kernel/src/selftest/` |
| isolation and security a hostile program can hit | `kernel/src/selftest/` |
| a few edge tests per hostile-input parser | the parser's `libs/` crate |
| a regression for each fixed bug with non-trivial logic | beside the code that had the bug |
| a small tooling smoke set | `xtask/` |

| The thing under test | Where it goes | How it runs |
|---|---|---|
| arithmetic, bit layout or parsing with no hardware | a `libs/` crate | `cargo test` |
| anything that needs a machine | `kernel/src/selftest.rs` and its children | `cargo xtask test-boot` |
| tooling a user runs | a host test in `xtask/` | `cargo test` |

A `no_std`, `no_main` kernel cannot host the `test` harness, so pure logic moves to a `libs/` crate
carrying `#![cfg_attr(not(test), no_std)]`, which `cargo test` builds for the host. A `libs/`
crate is a workspace member in `default-members`, so a bare `cargo test` covers it. A cargo
feature is a separate crate graph, and a build-only feature adds its graph to `CARGO_PASSES` in
`xtask/src/tasks.rs`. Tests never touch the environment; `Paths::discover` reads it, and no test
calls it.

A check that cannot fail still reads as evidence. Four rules keep a check able to fail:

- Write it from the claim, not from the implementation. Restating what the code does always
  passes.
- Choose inputs where the answers differ. Make values pairwise distinct, so no field or constant
  can stand in for another.
- Make it reach the code it is about. If the refusal under test is the second thing that could go
  wrong, the input has to get past the first.
- If its name says "every", "all" or "no", the body enumerates the domain rather than naming its
  members.

## Proofs

A proven crate is a `libs/` crate that Verus checks in place. Its `Cargo.toml` takes
`vstd.workspace = true` and sets `[package.metadata.verus] verify = true`, which is all it takes
to join: `cargo xtask verify` checks every crate so marked with the build `tools/verus/VERSION`
pins. A plain build erases all ghost code.

How a proof is laid out:

- `src/proof.rs`, reached as `pub mod proof`, holds the crate's one `verus! {}` module of
  specifications: the type invariants, the spec functions, each named for the statement it serves,
  and the lemmas. Exec code names them as `proof::name`. A spec function that reads the fields of
  a type in another module needs those fields `pub(crate)`; a type in the crate root needs nothing.
- A public function's `ensures` names only public specs and constants, so a private constant
  appears there as its literal value.
- A loop-free function stays plain Rust under `#[verus_spec(...)]`, with proof steps in `proof!`.
  Types, constants and impl blocks carry `#[verus_verify]`; on a type it makes Verus handle the
  derived `Debug`, which it cannot check. Constants in a nested module sit in that module's own
  `verus! {}`.
- A `pub const` of a type with private fields is an `exec const` with an `ensures` inside
  `verus! {}`, or carries `#[verus_spec(ensures ...)]`. A spec cannot read it, so a spec restates
  its value.
- A function with a loop is written whole inside `verus! {}` beside its plain siblings, because
  stable Rust refuses an attribute on a loop. `cargo fmt` does not format a macro body, so code
  there is formatted by hand in rustfmt's style: four-space indent, 100 columns, trailing commas in
  clause lists, and `requires`, `ensures`, `invariant` and `decreases` each on its own line. Clippy
  does lint that code, and a `proof {}` block may not end a block that returns nothing.
- Every function in a proven crate is verified: `cargo xtask verify` passes
  `--no-external-by-default`, so an item outside `verus! {}` with no attribute is still checked.
  The one exclusion is `mod fmt;`, marked `#[cfg_attr(verus_only, verifier::external)]` in the
  crate root, which holds the `Display`, `Error` and hand-written `Debug` impls Verus cannot check.
  Nothing else in a proven crate is marked external. `--no-cheating` refuses `assume`, `admit` and
  `external_body`.
- A `use` that only ghost code needs, such as `use vstd::prelude::*;` for `int` and
  `use_type_invariant`, carries `#[cfg(verus_only)]`; a plain build would find it unused.
- Proofs hold for a 32-bit and a 64-bit `usize`, so `bit_vector` steps state their shift bounds
  with `usize::BITS`. The `global size_of usize` directive is not used: its erased form fails
  `unused_braces`.

How state with an invariant changes:

- A `&mut self` postcondition names the result as `final(self)` and the start as `old(self)`.
- Verus checks a type invariant after every field assignment and after every call that borrows a
  field, and such a call must be `no_unwind`. State whose invariant ties several fields together
  changes through one function that takes `&mut` to each field; before the call, a lemma over the
  new values shows they satisfy the invariant. Before the next field assignment the method calls
  `use_type_invariant` again.
- A loop forgets what its invariant does not say: a value the loop leaves alone is stated as
  `*self == *old(self)`, and a fact proven before the loop and needed after it is in the invariant.

What keeps a proof stable:

- After changing one element of a sequence, assert the changed element and `assert forall` the
  unchanged ones in the terms the invariant uses. Give each conjunct of a quantified fact its own
  trigger.
- After a field assignment, spec functions of the new value are not linked to those of the old
  one; keep a `let ghost` copy and assert the equalities. A `choose` over `self` differs between
  two values with equal fields, so such a spec is defined over the field:
  `seen_in(entries, space)`.
- `bit_vector` sees only its own assertion: it names no spec function and no `let` binding, which
  it takes as `requires` instead, and it has no `usize::MAX`. A shift amount is a `u64` in code and
  spec alike: `1u64 << (bit as u64)`.
- Verus rejects `if let` chains and an unparenthesised `matches` beside `&&`, and reads `!has(...)`
  as its own `has` operator, so no spec function is named `has`.
- While iterating, `cargo verus focus -p CRATE -- --verify-function NAME --expand-errors` checks
  one function. It refuses `--no-cheating`, so a full `cargo xtask verify` decides.

The coding rules a provable crate follows:

1. State is fixed arrays and integers in private fields. Records name each other by index and
   generation; no reference is stored and nothing allocates.
2. Every public function is total, with no `requires`: it returns `Option` or `Result`, and a
   refusal changes no state. Unverified callers in `kernel/` and `wost` cannot meet a precondition.
3. Loops are `while` loops bounded by a length or a constant. No iterator adapters, closures or
   `impl IntoIterator` parameters; take slices.
4. No trait objects or generic traits on the proven surface. Formatting impls live in `fmt.rs`.
5. Overflow is explicit: `checked_*`, `wrapping_*`, `saturating_*` and `is_multiple_of` have
   specifications. `div_ceil`, `next_power_of_two`, `is_power_of_two`, `usize::trailing_zeros`,
   `abs_diff` and `div_euclid` have none, so the crate writes a local function.
6. Each bit trick sits in one small function. No `assert!`, unchecked indexing or `unwrap`.
7. One lock guards one state: no invariant of a proven crate spans two locks. A relation across
   locks is modelled in TLA+.

## Building

- Set `CARGO_TARGET_DIR` to put artifacts elsewhere; `xtask` does not read `build.target-dir` from
  a cargo config.
- Nothing host-specific goes in `rust-toolchain.toml`. Every host reads it, and rustup resolves its
  components against whichever host it runs on.

## Documentation

- Plain declarative sentences. The present tense says what is true now. The first sentence of a
  section says what it describes or decides.
- One idea per sentence, usually 12 to 22 words; split anything over 35.
- Name the thing: a path, a function, a register. Define a term once, where it first matters.
- Bold only for a table header or a labelled paragraph lead.
- No em or en dashes. Use a comma, a colon, parentheses, "to" for a range, or two sentences.
- No intensifiers or self-certification ("honest", "load-bearing", "robust", "the whole point"),
  no antithesis used as rhetoric, no narration of how a text was made, no praise of the design.
- No history in a design doc: no phase narration and no alternatives considered. Git and the
  decision log keep those.
- State a count only where a check keeps it true; otherwise point at the place that holds it.
- A decision is one entry in `docs/decisions.md`; an amendment is a new entry.
- Public text names no external runtime, scanner or person.
- ASCII only, LF line endings, prose wrapped at about 100 columns.

## Sources

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/): the default standard and its
  [checklist](https://rust-lang.github.io/api-guidelines/checklist.html).
- [Rust for Linux coding guidelines](https://docs.kernel.org/rust/coding-guidelines.html): the
  `// SAFETY:` and `# Safety` distinction and the namespace rule.
- [Asterinas OSTD](https://asterinas.github.io/book/ostd/): the privileged-framework shape `wost`
  follows, and its posture on soundness: argue the design and rely on review.
- [Oxide's Hubris](https://github.com/oxidecomputer/hubris): precedent for `xtask` over bare cargo
  in a multi-target build.
- [`rust-osdev/volatile`](https://github.com/rust-osdev/volatile): the argument behind
  `MmioRegion`, a typed region with a fixed set of operations.
- [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/): the closed list of
  reasons to write `unsafe`, and `unsafe_op_in_unsafe_fn`, which the workspace denies.
- [Rustonomicon, Working with Unsafe](https://doc.rust-lang.org/nomicon/working-with-unsafe.html):
  the sound-wrapper principle.
