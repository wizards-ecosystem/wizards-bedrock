# D-149 The ABI is extended before the filesystem is written

Status: Accepted, 2026-08-25. Amended by D-181.

**Decision.** One phase: Phase 6's first three slices are `013`'s work
(completions that carry handles, the time page, what happens to an operation
whose program has retired), and the filesystem is written against them, ring
native from its first `read`. `013` stops being a separate "Phase 6.5" and
becomes those slices.

**Rationale.** A caller freezes the interface it uses. Build `read()`
synchronous and add the ring form afterwards and the kernel has two doors to
the same operation for ever, which is what Phase 5 avoided by freezing the
handle table and the queue ABI *before* writing their callers. Storage is the
first external-I/O subsystem whose eventual implementation must defer while a
device works. D-164's Timer became the first operation that actually remains in
flight; the present Read has ring wire shape but polls the device synchronously
inside `RingEnter`, and was the first data caller, the one that forced the
submission from sixteen to thirty-two bytes. The cost is three slices before
anything is stored, the same cost D-19 charges for every charter. VFS first
would leave two doors; a separate platform phase would end with a machine that
still cannot store anything.

**Consequences.**

- Phase 6's charter is written after `012-research-notes-4`, and covers both
  halves.
- Phase 7 opens against an ABI whose Timer has already exercised genuine
  deferral and whose Read has exercised the storage wire shape. Read's device
  service is still polled inline; a deferred device state machine remains work.

**Left owed.** Nothing here says what the VFS *is*: the handle-per-file shape,
tmpfs, devfs, and whether the third-party format is FAT32 or ext2 are the
research pass's questions and the charter's answers.
