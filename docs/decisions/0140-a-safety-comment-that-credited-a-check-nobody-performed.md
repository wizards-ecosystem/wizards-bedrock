# D-140 A `SAFETY` comment that credited a check nobody performed

Status: Accepted, 2026-08-25.

**Context.** `mm::direct_pointer` turns a physical address into a direct-map
pointer. Its own doc is unambiguous:

> Arithmetic only. Whether the map actually *reaches* those bytes is the
> caller's to establish: `FirmwareMemory` checks the memory map, and a `Frame`
> carries it by construction.

Five `// SAFETY:` comments said the opposite — *"`direct_pointer` confirmed the
direct map covers all `PAGE_SIZE` bytes"*, *"which `direct_pointer` confirmed
the map covers in full"* — in `entry_pointer`, both halves of `Segment`'s
copy, and twice in the heap. One of them was in a function that never calls
`direct_pointer` at all.

Nothing was unsound. The real ground was always there and always sufficient:
**the direct map covers every frame the allocator manages**, established at
`mm::init` and sampled by a boot check, and every address these sites pass is a
frame the caller owns. What was wrong was the *stated* reason.

That matters more than it sounds. D-139 found two doors — `lookup` and `bind` —
walking a reclaimed space's root, which is physical frame zero. A reader
auditing those walks arrives at `entry_pointer`, reads that `direct_pointer`
confirmed the coverage, and stops. **The false comment is what made the
unguarded walks look justified.**

**A second finding of the same shape.** `Root` is unforgeable — only
`task::address_space_of` mints one — and that was taken to settle what it
authorises. It does not. A caller can hold one across the thread's exit;
`collect` then reclaims the space and returns the root frame to the allocator,
which may hand it to a **program**. The walk then reads ring-3-authored bytes
as top-level page-table entries: the arbitrary physical read the type exists to
prevent, reached by staleness instead of forgery. Every sibling evidence type
in that module — `Binding`, `MappedStack`, `StackArena` — is deliberately not
`Copy` with a comment saying why; `Root` carried the same class of claim with
none of it.

**Decision.**

The five comments now state the real ground: provenance plus boot-established
coverage, with `direct_pointer`'s actual contribution named as what it is —
arithmetic, and a refusal on overflow.

`Root` records the slot it was minted from and gains `is_live`, which asks the
task table again. `translate_in_root` refuses a stale one. It stays `Copy`:
copying is harmless once the *reading* is guarded, and reading is the only
thing it authorises — which is a different answer from `MappedStack`'s and
worth being explicit about rather than applying a rule.

A `STALE_ROOT_REFUSALS` tally lives inside the guard, and the check drives a
real program to completion, collects it, and requires both the refusal and the
count. Deleting the guard turns it red.

**Rationale.** The lesson is about what a `// SAFETY:` comment is *for*. This
project requires one on every unsafe operation, and the point is not that
something was thought about — it is that the next reader can check the
reasoning. A comment naming the wrong justification passes review exactly as
well as one naming the right one, and is worse than none: it tells the reader
the question has been answered and points them at the answer.

The `Root` half is D-125's lesson a second time — *evidence has a date* — and
it is the third instance this week: `MappedStack` outliving its mapping, a
reclaimed space still answering, and now a root outliving its thread. The
common shape is a value that was true when minted, in a system where the thing
it describes can be **given away**. Any such value needs a freshness question,
and unforgeability does not supply one.

**Consequences.**

- Five `// SAFETY:` comments state their real ground. No code changed for them,
  and that is the point: the comments were the defect.
- `Root` carries its slot; `translate_in_root` re-asks. 142 checks.
- `paging::stale_root_refusals` is the third tally-inside-a-guard, after
  `CLEARED_ON_DROP` (D-123) and `RECLAIMED_REFUSALS` (D-139). All three exist
  because the guard's absence is invisible in the answer.

**Left owed.** Nothing checks that a `// SAFETY:` comment names a real reason;
nothing can. `xtask/src/unsafety.rs` counts the lines and the lints require the
prose to exist. Whether it is *true* is a reading, and this entry is the record
of five that were not.

The audit's other latent items — the handle arena's `take`/`insert` race and
`mem::with_active_space` handing safe code `&mut` the kernel's own space — are
the same family and are not closed here.
