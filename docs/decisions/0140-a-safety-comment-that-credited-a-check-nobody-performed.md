# D-140 A `SAFETY` comment that credited a check nobody performed

Status: Accepted, 2026-08-25.

**Decision.** The `// SAFETY:` comments that said *"`direct_pointer` confirmed
the direct map covers all `PAGE_SIZE` bytes"* state their real ground:
provenance (every address these sites pass is a frame the caller owns) plus
boot-established coverage (**the direct map covers every frame the allocator
manages**, established at `mm::init` and sampled by a boot check), with
`direct_pointer`'s actual contribution named as what it is: arithmetic, and a
refusal on overflow. Its own doc says so:

> Arithmetic only. Whether the map actually *reaches* those bytes is the
> caller's to establish: `FirmwareMemory` checks the memory map, and a `Frame`
> carries it by construction.

`Root` records the slot it was minted from and gains `is_live`, which asks the
task table again; `translate_in_root` refuses a stale one. A `Root` held across
its thread's exit names a root frame `collect` may have handed to a
**program**, so an unguarded walk reads ring-3-authored bytes as page-table
entries. It stays `Copy`: copying is harmless once the *reading* is guarded,
and reading is the only thing it authorises, a different answer from
`MappedStack`'s. A `STALE_ROOT_REFUSALS` tally lives inside the guard, and the
check drives a real program to completion, collects it, and requires both the
refusal and the count.

**Rationale.** The point of a `// SAFETY:` comment is that the next reader can
check the reasoning. A comment naming the wrong justification passes review
exactly as well as one naming the right one, and is worse than none: it tells
the reader the question has been answered and points them at the answer. The
false comment is what made D-139's unguarded walks look justified. `Root` is
D-125's lesson again, *evidence has a date*: a value true when minted, in a
system where the thing it describes can be **given away**, needs a freshness
question, and unforgeability does not supply one.

**Consequences.** Nothing was unsound in the comments' code; the comments were
the defect.

**Left owed.** Nothing checks that a `// SAFETY:` comment names a real reason;
nothing can. `xtask/src/unsafety.rs` counts the lines and the lints require the
prose to exist. Whether it is *true* is a reading, and this entry is the record
of five that were not.

The audit's other latent items (the handle arena's `take`/`insert` race and
`mem::with_active_space` handing safe code `&mut` the kernel's own space) are
the same family and are not closed here.
