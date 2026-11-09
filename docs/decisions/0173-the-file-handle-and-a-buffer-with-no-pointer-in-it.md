# D-173 The file handle, and a buffer with no pointer in it

Status: Accepted (2026-08-25) (amended by D-181)

**Context.** `014` left three decisions for this slice: what a `File` handle's
object is, whether `read` takes a length or a provided buffer, and — implicitly
— how a program comes to hold a file at all.

D-149 put the ABI slices first because a caller freezes the interface it uses.
Read is the first storage/data caller: it needs a handle, an offset, a buffer
and a length. The submission entry was sixteen bytes and is thirty-two now,
which is cheap today and an ABI break tomorrow. Timer, not Read, was the first
operation that genuinely waited. D-200 later moved Read service out of its
`RingEnter` trap and into one bounded worker.

**Decision.**

**A `File` handle's object is the thinner of the two.** `vfs::File` names a
backend and a node within it — not an inode-like structure the VFS owns. The
fatter one is what a write path and a page cache both want, and this phase has
neither; building it now would be building for a caller that does not exist
(D-41).

**`read` takes a provided buffer, and the buffer is part of the ring.** `012`
§8 warns that `-ENOBUFS` at completion is the same late-failure shape D-151
refused for handles, and the usual answer is to validate a caller's pointer at
submission. This kernel does something simpler and stronger: a third shared
page is the data area, so the buffer is always mapped, always the right size,
and its bounds are two comparisons. **There is no user pointer to validate, no
SMAP window to open and no fault to recover.**

The cost is a copy — device → kernel frame → data area, where a caller's own
buffer could have been the DMA target — and it is named rather than hidden.
Zero-copy needs the memory-object work of a later phase, and paying one copy
for a boundary with no pointer in it is the right trade until then.

**`OpenAt` is how a program comes to hold a file**, and its rights are
**derived, never asked for**. Opening a directory preserves the same-type
rights unchanged. Opening a file preserves only universal rights already held
and derives `READ` from `TRAVERSE`, because traversal authorized reaching the
named bytes. That cross-type derivation is not a subset relation between the
two raw bitsets; the non-escalation rule is that it never recreates a universal
right or admits a type-specific power with no authorizing operation. `WRITE`
is not granted because there is no write path, and a program wanting fewer
rights can `Duplicate` with fewer.

A device is refused. What a program will hold for one is slice 4's `Device`
capability, and putting it in a table is Phase 7's (D-172).

**Rationale, and three checks that could not fail.**

*A read without the right to read.* The refusal table was built entirely from
`OpenAt`, so nothing exercised `Read`'s rights check at all — deleting it left
the boot green. A file handle held without `READ` is in the table now.

*A zero-length buffer.* Refused by `data_bounds`, and the check covered it — but
only through `OpenAt`, where a zero-length path is also refused by the path
splitter. The two refusals are different and the check now distinguishes them.

*The buffer bound checked before the read rather than after.* This is the one
worth recording. Removing it changes **no output**: `Shared::write_at` bounds
the copy anyway, so the completion carries the same status either way. What it
changes is *when* — a bound checked after the read is `-ENOBUFS` with the work
already done, which is precisely the shape this design refuses.

Making it observable took a counter on the driver: the controller's command
identifier advances once per command, so "the device was not asked" is a
number. The check submits a readable handle with a buffer that runs off the end
of the data area and requires the count **unchanged**. Every refusal that
happens before the read is now visible as a device that was never told.

**Consequences.**

- At this decision four transcriptions of the entry layout existed: the kernel
  parser/serializer, C header, emitted ring-3 program and test helpers. The host
  check compared only total sizes and stride, not fields; D-181 found the gap.
  The repair uses one kernel offset map for parser, serializer and helpers,
  derives C offsets, widths and sizes, and checks the emitted ELF's actual
  operands plus entry base and count. A persistent mutation changing submission
  `argument` from offset 24 to 8 must make the program-ring check red.
- Three raw rights were added: `TRAVERSE`, `ENUMERATE`, `MODIFY`. The current
  masks admit `READ` for File and `TRAVERSE | ENUMERATE` for Directory. `WRITE`
  remains meaningful for Channel but no File admits it; no Directory admits
  `MODIFY`, and no type admits `EXECUTE` or `MAP`. Leaving those raw bits in
  place avoids renumbering later without pretending an unenforced power exists.
- **Two rights tests were restated lists and are now derived.** One named the
  eight rights that existed; the other named `1 << 12` as an undefined bit,
  which `TRAVERSE` then became. Both read the source now (D-161).
- A third test's premise was simply wrong once `ALL` grew: it asserted that
  every right this kernel defines is meaningful for a memory object, which was
  true only while `ALL` was the universal four plus the memory ones. A memory
  object has no names in it.
- `Submission::simple` and `Submission::about` exist so that eighteen call
  sites did not each grow four zeroes. A zero handle is a *value*, and writing
  it out invites a reader to think it matters.
- **A ring-3 program still cannot reach any of this**, and that is the slice's
  named limit rather than an omission: `spawn` gives a program exactly one
  handle — its bootstrap channel (D-111) — and receiving a directory handle
  over that channel needs `sys_channel_receive`, which was called OWED under
  D-114's former vocabulary. D-181 states the live fact directly: there is no
  ring-3 receive operation. Giving a program a second handle at spawn would
  break the invariant that makes pillar 1 checkable, so it waits for the
  channel.
