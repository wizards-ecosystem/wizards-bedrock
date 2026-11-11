# D-95 The second audit: a fix that introduced undefined behaviour, and a wait that could sleep for ever

Status: Accepted, 2026-08-24.

**Decision.** A second adversarial pass covered slice 6 and D-94's own repairs.
Its fixes:

- **The TSS store is aligned.** `TaskState` is `#[repr(C, packed(4))]` and
  `IST1` sits at offset `0x24`, four-mod-eight, so D-94's eight-byte store into
  an eight-aligned segment was **misaligned, which is undefined behaviour**. The
  segment is placed four-mod-eight, with four bytes of padding in front.
  `x86-tables` pins both facts: the offset is `0x24`, checked against
  `offset_of!`, and it is four-mod-eight, *why* the padding exists.
- **`WaitQueue::wait_unless` replaces `Event::wait`'s read-then-block**, which
  could sleep for ever when a signal landed between the read and the block.
  Registration and the re-check happen under the queue lock a waker must take,
  so a signal cannot land between them. It clears the registration bit on a
  non-wake return.
- **D-94's rollback invalidates what it clears**: one `invlpg` per cleared
  entry when the space is the installed one, because `map_owned` then hands the
  frames back to the allocator.
- **`ARENA` takes no allocation under its leaf guard**, which rebuilt
  `ARENA → HEAP → FRAMES`, the order D-85 deleted. Growth is reserved outside
  the lock; `Drop` sizes its buffer from `live` before taking it.
- **`insert`'s recovery path frees the slot it claimed**; `close(0)` was a
  no-op, since zero can never decode.
- **`Mixer::for_table` adds an odd constant after the multiply**, so table zero
  is not the identity, and the crate no longer claims a small integer "is never
  a valid handle".
- **Rights are checked against object type.** D-78's one rule is that a right
  names a distinction the kernel makes *about that object*; an event has no
  pages. `Rights::are_meaningful_for` and a per-type mask make the rule
  enforceable; `insert` refuses the rest.

Three checks could not fail and were repaired. *a handle does not cross
tables* forges a handle with the other table's mixer, so deleting the ownership
comparison turns it red; before, a foreign handle decoded outside the arena and
never reached the comparison. The `x86-tables` layout test checks the
transcribed offset against `offset_of!`. `a_malformed_value_never_decodes` no
longer skips the values where its property is interesting. The wait-window
check runs the wait on a spawned thread and calls `wait_unless` directly, so a
lost wakeup is a timeout in the *observer*, red rather than hung.

**Rationale.** A fix is not exempt from the standard it was written to uphold:
D-94's TSS repair was green, checked, and wrong, under a `SAFETY` comment that
argued the opposite. **The race a check steps around is the race nothing
tests**: the event check waits for the waiter to block, right for D-49's
reason, and leaves the window unexercised. One CPU cannot drive it (D-70 rule
6), so slice 9's model, not a self-check, will settle it.

**Consequences.**

- `011-userspace.md`'s lock-order section gains `ARENA`.
- **Two findings are carried, not fixed.** `undo` can still clear an entry that
  existed before the call, where a range overlaps an identical pre-existing
  mapping. Only the direct map's outward rounding produces that, and it aborts
  the whole space, but the next caller recovering from a `map_range` error will
  meet it. `NotYours` versus `Stale` is an occupancy oracle over a global arena:
  a table can learn which slots another holds. Zircon collapses both to one
  error for that reason. This is a decision for the phase that has a syscall to
  surface them through.
