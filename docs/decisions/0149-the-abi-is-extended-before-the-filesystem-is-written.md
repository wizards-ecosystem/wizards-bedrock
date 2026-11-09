# D-149 The ABI is extended before the filesystem is written

Status: Accepted, 2026-08-25. Amended by D-181.

**Context.** `000`'s roadmap makes Phase 6 "VFS + storage": a VFS over handles,
tmpfs and devfs, and a read-only third-party format over NVMe. `013` was
drafted as a separate "Phase 6.5" to follow it, holding three constraints on
what a program may ask for.

The ordering is the decision, and it is not cosmetic. Storage is the first
external-I/O subsystem whose eventual implementation must defer while a device
works. D-164's Timer became the first operation that actually remains in
flight; the present Read has ring wire shape but polls the device synchronously
inside `RingEnter`. Read is instead the first storage/data caller that needs a
handle, offset, buffer and length together, and therefore freezes the entry
layout it uses.

**Options.**

1. **VFS first, platform after**, as the roadmap reads. Fastest to a machine
   that can open a file.
2. **Platform as its own phase, VFS after.** Cleanest separation; the machine
   still cannot store anything at the end of it.
3. **One phase: the ABI extensions as slices 1–3, the filesystem on top.**

**Decision.** Option 3. Phase 6's first three slices are `013`'s work —
completions that carry handles, the time page, what happens to an operation
whose program has retired — and the filesystem is written against them, ring
native from its first `read`.

**Rationale.** Build `read()` synchronous and add the ring form afterwards and
the kernel has two doors to the same operation for ever. That is not a
hypothetical: it is what `011` records Phase 5 avoiding by freezing the handle
table and the queue ABI *before* writing their callers, on the argument that
both get frozen by their first caller. Echo was the first queue caller, Timer
was the first deferred one, and Read was the first data caller that forced the
submission from sixteen to thirty-two bytes.

The cost is a slower start — three slices before anything is stored — and it is
the same cost D-19 charges for every charter.

**Consequences.**

- `013` stops being a phase and becomes Phase 6's first three slices. Its own
  framing said it was "not a phase in the sense the others are"; this makes
  that true rather than apologetic.
- Phase 6's charter is written after `012-research-notes-4`, and covers both
  halves.
- Phase 7 opens against an ABI whose Timer has already exercised genuine
  deferral and whose Read has exercised the storage wire shape. Read's device
  service is still polled inline; a deferred device state machine remains work.

**Left owed.** Nothing here says what the VFS *is* — the handle-per-file shape,
tmpfs, devfs, and whether the third-party format is FAT32 or ext2 are the
research pass's questions and the charter's answers.
