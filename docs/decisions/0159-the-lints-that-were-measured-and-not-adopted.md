# D-159 The lints that were measured and not adopted

Status: Accepted, 2026-08-25.

**Decision.** Beyond the default set the workspace denies three clippy lints,
`undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block` and
`missing_safety_doc`. The others were measured across the whole tree, host and
kernel targets, because `005` says **measure what a tool does; do not reason
about it**. None is adopted, and this entry exists so the measurement is not
repeated.

| Lint group | Hits | What they were |
|---|---|---|
| `clippy::pedantic` | 59 in six `libs/` crates | 17 cast, 7 `must_use_candidate`, 4 `doc_markdown`, the rest style |
| `clippy::nursery` | 4 | one real `redundant_clone`, in a test |
| `clippy::indexing_slicing` | 118 in `kernel` + `wost` + `libs` | every one behind a `get`/`take` that had already proved the length |
| the cast family | 66 in `kernel` + `wost` | sign-extension in `x86-paging` is the canonical-address arithmetic itself |

**Rationale.** Not one hit is a defect. Every sampled indexing site indexes a
slice the function has just bounds-checked (`elf64` reads `field[0]` only after
`take(bytes, at, 2)?`), so the lint would mean 118 suppressions asserting a
proof exists somewhere above, and `005` records that a suppression which reads
as checked and is not falsifiable is worse than the `allow` it replaced. The
cast lints report the mechanism: `((address as i64) << 16 >> 16) as u64` *is*
`is_canonical`.

**Consequences.**

- The one genuine finding, a `redundant_clone` in an `acpi-tables` test, is not
  worth a lint that costs 118 suppressions, and is left.
- If a future phase parses a new hostile format, the measurement is cheap to
  redo and this table is the baseline.
- The measurement confirmed the tree has no `.unwrap()` in `kernel` or `wost`
  outside the self-test, and its sixteen panicking calls are all deliberate
  halts on a broken kernel invariant.
