# D-159 The lints that were measured and not adopted

Status: Accepted, 2026-08-25.

**Context.** `005` says **measure what a tool does; do not reason about it**.
The workspace denies three clippy lints beyond the default set —
`undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`,
`missing_safety_doc` — and nothing recorded why the rest were not on. "It would
be noisy" is a guess, and the rule says to go and look.

**Decision.** Measured across the whole tree, host and kernel targets. None is
adopted, and this entry exists so the measurement is not repeated.

| Lint group | Hits | What they were |
|---|---|---|
| `clippy::pedantic` | 59 in six `libs/` crates | 17 cast, 7 `must_use_candidate`, 4 `doc_markdown`, the rest style |
| `clippy::nursery` | 4 | one real `redundant_clone`, in a test |
| `clippy::indexing_slicing` | 118 in `kernel` + `wost` + `libs` | every one behind a `get`/`take` that had already proved the length |
| the cast family | 66 in `kernel` + `wost` | sign-extension in `x86-paging` is the canonical-address arithmetic itself |

**Rationale.** Not one of them is a defect. The indexing count is the
interesting one: 118 sites, and every one sampled indexes a slice the function
has just bounds-checked — `elf64` reads `field[0]` only after `take(bytes, at,
2)?`, `acpi-tables` reads `head[15]` only after `bytes.get(..V1_LEN)`. The lint
cannot see the proof, so adopting it would mean 118 suppressions asserting that
a proof exists somewhere above. That is worse than nothing: `005` already
records that a suppression which reads as checked and is not falsifiable is
worse than the `allow` it replaced.

The cast lints are the same shape. `((address as i64) << 16 >> 16) as u64` *is*
`is_canonical`; the sign loss the lint reports is the mechanism.

**Consequences.**

- No lint added, no suppression added, no code changed by this entry.
- The one genuine finding — a `redundant_clone` in an `acpi-tables` test — is
  not worth a lint that costs 118 suppressions, and is left.
- If a future phase parses a new hostile format, the measurement is cheap to
  redo and this table says what the baseline was.
- What *is* worth keeping is the shape the measurement confirmed: the tree has
  no `.unwrap()` in `kernel` or `wost` outside the self-test, and its sixteen
  panicking calls are all deliberate halts on a broken kernel invariant — a
  refused yield, a resumed dead thread, a second boot handoff.
