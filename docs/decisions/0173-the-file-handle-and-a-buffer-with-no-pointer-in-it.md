# D-173 The file handle, and a buffer with no pointer in it

Status: Accepted, 2026-08-25. Amended by D-181.

**Decision.** Read is the first storage/data caller and freezes the entry it
uses (D-149): it needs a handle, an offset, a buffer and a length, so the
submission entry grows from sixteen bytes to thirty-two, cheap today and an ABI
break tomorrow. Timer, not Read, was the first operation that genuinely waited.
D-200 later moved Read service out of its `RingEnter` trap and into one bounded
worker.

**A `File` handle's object is the thinner one.** `vfs::File` names a backend
and a node within it, not an inode-like structure the VFS owns. The fatter one
is what a write path and a page cache want, and this phase has neither (D-41).

**`read` takes a provided buffer, and the buffer is part of the ring.** `012`
§8 warns that `-ENOBUFS` at completion is the same late-failure shape D-151
refused for handles. A third shared page is the data area, so the buffer is
always mapped, always the right size, and its bounds are two comparisons.
**There is no user pointer to validate, no SMAP window to open and no fault to
recover.** The cost is a copy, device to kernel frame to data area, named
rather than hidden; zero-copy needs a later phase's memory-object work.

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

**Rationale.** Every refusal must happen before the device is asked: a bound
checked after the read is `-ENOBUFS` with the work already done, the shape this
design refuses. The controller's command identifier advances once per command,
so the check submits a readable handle with a buffer that runs off the end of
the data area and requires the count unchanged; every refusal before the read is
visible as a device that was never told. The refusal checks also hold a file
handle without `READ`, and distinguish `data_bounds`' zero-length refusal from
the path splitter's.

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
- The rights tests that named the existing rights and an undefined bit are
  derived from the source now (D-161), and a memory object is not asserted to
  find every right meaningful.
- `Submission::simple` and `Submission::about` keep call sites from writing out
  zero handles: a zero handle is a value, and writing it out invites a reader
  to think it matters.
- **A ring-3 program still cannot reach any of this**, and that is the slice's
  named limit rather than an omission: `spawn` gives a program exactly one
  handle, its bootstrap channel (D-111), and receiving a directory handle over
  that channel needs `sys_channel_receive`, which was called OWED under D-114's
  former vocabulary. D-181 states the live fact directly: there is no ring-3
  receive operation. Giving a program a second handle at spawn would break the
  invariant that makes pillar 1 checkable, so it waits for the channel.
