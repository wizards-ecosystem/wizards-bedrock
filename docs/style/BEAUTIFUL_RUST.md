# Beautiful Rust in Bedrock

The style standard for Rust in this tree. [`docs/005-code-conventions.md`](../005-code-conventions.md)
stays the authority on crate boundaries, kernel comments, and tests. This file
is the mechanical layer: what clippy now warns on, what it must not "fix", and
how an unsafe block is written. Agents and contributors follow both.

The lint selection is adapted from [BeautifulRust](https://github.com/morganlinton/BeautifulRust)
(MIT, Copyright (c) 2026 Morgan Linton): its `rust-best-practices` and
`rust-review` skills, and the clippy table its `scripts/check-lints.sh` proves
fires. This file is Bedrock's summary of those rules. It is not a copy of the
skills. Where a skill and this kernel disagree, the paragraph below says which
wins.

## What to run

Before handing Rust back:

```text
cargo fmt
cargo clippy --all-targets --locked
cargo kclippy --locked
```

`cargo clippy` lints the host half (`default-members`: `libs/` and `xtask`).
`cargo kclippy` lints `wost` and `wizard-kernel` for `x86_64-unknown-none`.
`cargo clippy --workspace` does not work here. Never set `RUSTFLAGS`; it
replaces the kernel code-model flags in `.cargo/config.toml`. Lint flags go
after `--`.

Warnings do not fail `cargo check`, `cargo build`, or those two clippy
commands. They do fail `cargo xtask clippy` and `cargo xtask ci`, because that
gate passes `-D warnings`. The new lints are **warn on purpose** so other
branches still build. Clearing a warning is the job of the crate's owner;
silencing the whole group to get a green gate is not.

`xtask` does not inherit `[workspace.lints]` (D-22). Its rustdoc link denies
are its own. A clippy nit inside `xtask/` is invisible to this table until
that opt-in changes.

## Levels

Configured in the root `Cargo.toml` `[workspace.lints]` and `clippy.toml`.
Every member except `xtask` already has `[lints] workspace = true`.

**Stay deny** (D-22, unchanged):

| Lint | Why it stays deny |
|---|---|
| `unsafe_code` | `wost` is the only crate that may write `unsafe`. `kernel` denies it; `libs/` forbid it in source. |
| `unsafe_op_in_unsafe_fn` | The body of an `unsafe fn` is not an unsafe block. Each operation gets its own block and its own comment. |
| `unused_must_use` | A discarded `Result` is a dropped failure. |
| `undocumented_unsafe_blocks` | Every unsafe block and unsafe impl carries a `// SAFETY:` comment. |
| `multiple_unsafe_ops_per_block` | One unsafe operation per block, so the comment covers one thing. |
| `missing_safety_doc` | Every `pub unsafe fn` has a `# Safety` section. |
| `broken_intra_doc_links`, `private_intra_doc_links`, `redundant_explicit_links` | Doc links rot silently. These fire under `cargo xtask doc`. |

BeautifulRust warns on the two unsafe-block lints. Bedrock already denies
them. The deny wins.

**Warn** (new, from the tested table plus `clippy::pedantic` and `clippy::all`):

- `unwrap_used`, `panic_in_result_fn`, `missing_panics_doc`, `missing_errors_doc`, `missing_assert_message`
- `allow_attributes`, `allow_attributes_without_reason`
- `fn_params_excessive_bools` (threshold 0: any `bool` parameter), `struct_excessive_bools`
- `clone_on_ref_ptr`, `redundant_clone`, `rc_buffer`, `exhaustive_enums`
- `string_add`, `wildcard_imports`, `dbg_macro`, `todo`, `print_stdout`, `print_stderr`
- `cast_possible_truncation` and the rest of `pedantic` that is not listed under Allows
- `missing_docs`, `missing_debug_implementations`, `unreachable_pub`, `rust_2018_idioms`

`clippy::nursery` stays off. It changes every toolchain release, and the pin
is exact so a toolchain bump is a decision (D-24). The blanket `restriction`
group stays off.

## Allows, and why

These are the places BeautifulRust would warn and this kernel does not. Do not
turn them back on without a decision.

| Lint | Why it is allow |
|---|---|
| `expect_used` | `expect("why this cannot fail")` is the sanctioned panic for a broken invariant. Recoverable failure is still `Result`. Several `libs/` crates already `deny` this on non-test code; that attribute is stricter and stays. |
| `as_conversions` | A ban on every `as` fights port widths, MMIO widths, and page-table bit packing. Prefer `From` / `TryFrom` when the conversion can lose a value the caller could have avoided. |
| `cast_sign_loss`, `cast_possible_wrap` | `((address as i64) << 16 >> 16) as u64` is the canonical-address arithmetic, not a defect (D-159). A warning on that pair asks someone to "fix" the mechanism. |
| `cast_possible_truncation` is **warn**, not allow | Narrowing to a register width is often real, and it is also how a value silently loses bits. Keep the warning. On a hardware store whose width is the register, `#[expect]` with a reason, or `uN::try_from`, whichever is honest. |
| `module_name_repetitions`, `module_inception`, `enum_glob_use`, `must_use_candidate` | BeautifulRust's tested config turns these off. A leaf module named after its item is the house layout. `use Enum::*` inside the function that matches on it is fine. |
| `enum_variant_names` | The lint wants variants prefixed with the type name (`ParseError::ParseErrorEmpty`). That repeats a namespace the path already carries. Do not "fix" it. |
| `indexing_slicing` | Not enabled. D-159 counted 118 hits, each behind a `get` or `take` that had already proved the length. The lint cannot see the proof. |
| `missing_docs_in_private_items` | Not enabled. It documents every private helper and produces noise. |

`avoid-breaking-exported-api` is false in `clippy.toml`. Clippy's default
hides pedantic findings on exported items so a published crate can avoid a
breaking-change nit. This workspace is one product; `exhaustive_enums` should
be visible on the public error types. `#[non_exhaustive]` belongs on error
enums. A closed hardware or protocol encoding stays exhaustive: a new variant
there is a bug, not a compatible extension. `#[expect(clippy::exhaustive_enums, reason = "closed encoding")]` on that type.

Tests may `unwrap`, `expect`, `panic`, `dbg`, and print (`clippy.toml`). So
may const evaluation: a panic there fails the compile. Kernel self-test code
is a cargo feature, not `#[cfg(test)]`, so those allows do not cover it.

## Unsafe

Three valid reasons to write `unsafe` (D-41), and only in `wost`:

1. a novel abstraction the language cannot express;
2. performance, where a checked form exists and a measurement shows it costs too much (unused so far);
3. FFI or the platform: an instruction, a register, a bootloader ABI.

A safe function that can cause undefined behaviour through safe calls is
unsound. Mark a function `unsafe` only when misuse can cause undefined
behaviour, not because it is dangerous.

`# Safety` on the `unsafe fn` states the precondition the caller must meet.
`// SAFETY:` on the block states why this call meets it. One is the contract,
the other is the evidence. Clippy checks that both exist; it does not check
that either is true.

```rust
/// Stores `value` at `index`.
///
/// # Safety
///
/// `index` must be in range for the region, and the region must be mapped
/// with the width of `value`.
pub unsafe fn write_u32(&self, index: usize, value: u32) {
    let ptr = self.ptr_at(index);
    // SAFETY: `ptr_at` rejects an out-of-range or misaligned index, and the
    // caller has the mapping named in the contract.
    unsafe {
        ptr.write_volatile(value);
    }
}
```

One unsafe operation in the block. Two operations are two blocks and two
comments. An `unsafe impl` gets the same `// SAFETY:` comment. Do not use
`transmute` to invent a lifetime or to turn an integer into an enum.

Inside a macro, keep `#[allow(unsafe_code, reason = "...")]` rather than
`#[expect]`. An unfulfilled `expect` is not checked in a macro expansion, so
the attribute reads as a proof and is not one (D-131). The entry glue in
`wost` is that case: `kernel` denies `unsafe_code`, and the macro expands an
allow into the binary crate. `forbid` cannot be lifted that way, which is why
`kernel` uses `deny`.

## Suppressions

Fix the lint when the fix is honest. When the lint is wrong for one item:

```rust
#[expect(clippy::cast_possible_truncation, reason = "the BAR is a 32-bit register")]
```

Put it on the narrowest item. `expect` fails the build once the lint stops
firing, so a stale suppression gets deleted. `#[allow]` without a reason warns.
`#[allow]` with a reason still warns under `allow_attributes`, outside macros,
because `expect` is the form that expires.

Do not write `#![deny(warnings)]` in a crate. New toolchains add warnings and
break the build for a reason that is not a commit.

## What the skills add, beyond the lints

Short form of the rules worth following when the lint does not fire. 005
already has the kernel-specific versions of several of these.

- Model absence with `Option`, failure the caller can handle with `Result`,
  and a broken invariant with `expect` or `assert` whose message contains the
  values. No `unwrap` outside tests, consts, and `main`-shaped host code.
- `Display` for an error is a lowercase clause with no trailing period.
  Library errors implement `core::error::Error`. No `String` error, no
  `anyhow` in a `libs/` or `wost` signature. `xtask` uses `anyhow` and
  `.context(...)` on every fallible call.
- Take `&str`, `&[T]`, or `&T`, not `&String`, `&Vec<T>`, or `&Box<T>`. Take
  ownership by value when the function stores the value. No `bool` parameter
  in a signature another crate, or another module, calls.
- `as_` borrows, `to_` may allocate, `into_` consumes. No `get_` prefix on a
  getter. Do not repeat the module name inside the type.
- Derive `Debug` on public types. A `Debug` impl in the kernel must not block.
- `///` on public items; the first sentence is one line. Sections, when they
  apply, go summary, body, `# Examples`, `# Errors`, `# Panics`, `# Safety`.
  Examples live in `libs/` only, where doctests run (D-41). `wost` and
  `kernel` are cross-compiled, so an example there would rot.
- Comments say why, not what. The comments worth writing record a decision
  the code cannot show.
- No glob imports, except `use super::*` in a test module and `use Enum::*`
  in the function that matches on it. No crate prelude of our own.
- Format with the stable `rustfmt.toml` (`max_width` 100, Unix newlines,
  field-init and `?` shorthand, style edition 2024). Do not add nightly
  rustfmt options. Google's `hex_literal_case` and `format_macro_matchers`
  are unstable on this toolchain and would format differently, or not at all,
  across hosts.
- Shared ownership (`Rc`, `Arc`, `RefCell`) is a design, not a way out of the
  borrow checker. Clone small data on purpose; do not clone only to end a
  borrow. Destructors do not panic, fail, or block.

## Relationship to D-159

D-159 measured `pedantic`, `nursery`, `indexing_slicing`, and the cast family,
found no defect, and adopted none of them. That measurement is why the cast
and indexing lints above are allow or off, and why nothing here is deny.
Adopting the rest of the tested table at **warn** is the step D-159 did not
take: the warnings are visible, and they are not yet a gate. The decision log
still says the groups were not adopted. Amending that entry, or writing the
next one, is open.

## Open questions

- Should the decision log record this warn rollout as an amendment to D-159?
  This stream did not edit `docs/004-decision-log.md`.
- `cargo xtask clippy` denies every warning. Should that denial wait until the
  per-crate counts in `handoff/streams/01-lint-foundation.md` are cleared, or
  should the streams clear them before the branches combine? Changing the
  gate means editing `xtask`, which this stream does not own.
- Source preview CI lints the host half and the shipped kernel only. It does
  not lint the self-test or `refuse-pci` graphs, and it does not pass
  `-D warnings`, so the 20-minute preview budget (D-441) stays a preview.
  The local gate still lints every graph.
