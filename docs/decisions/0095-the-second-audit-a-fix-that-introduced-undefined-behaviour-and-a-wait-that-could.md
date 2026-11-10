# D-95 The second audit: a fix that introduced undefined behaviour, and a wait that could sleep for ever

Status: Accepted, 2026-08-24.

**Context.** A second adversarial pass covered slice 6 and D-94's own repairs.
From a green baseline (94 machine checks, every host test passing) it found
eleven defects, three worse than anything in the first.

**The worst was in a D-94 fix.** D-94 replaced a whole-`TaskState`
`write_volatile` with a targeted eight-byte store, arguing from the field's
width that the result was atomic. But `TaskState` is `#[repr(C, packed(4))]` and
`IST1` sits at offset `0x24`, four-mod-eight. In a segment on an eight-byte
boundary that field is *four*-byte aligned, so the store was **misaligned, which
is undefined behaviour**, and its atomicity held only by the linker's choice.
The replaced code was aligned and only observably torn, so the fix made it worse
in the one dimension it was about.

The repair places the segment four-mod-eight, with four bytes of padding in
front, so `0x24` lands on a boundary. `x86-tables` now pins both facts: the
offset is `0x24`, checked against `offset_of!`, and it is four-mod-eight, *why*
the padding exists.

**Second: `Event::wait` could sleep for ever, and three docs said it could
not.** It read the condition, then called the wait queue to register and block.
A signal landing in the gap woke an empty queue, and the waiter then blocked
with nothing left to wake it. A sticky signal did not cover this: the state is
re-read only once a block *returns*, which happens only on a wake. Stickiness
rescues only a case that stops arising once an idle thread exists.

`WaitQueue::wait_unless` replaces it. Registration and the re-check happen under
the queue lock a waker must take, so they are one step and a signal cannot land
between them. It also clears the registration bit on a non-wake return, which
the old path never did, leaving a bit naming a thread no longer waiting there.

**Third: D-94's rollback cleared live page-table entries without invalidating
them.** `undo` runs against the *active* space, and `map_owned` then hands the
frames back to the allocator. That is D-94's stale-TLB hazard for `unmap`,
arriving by the other door. The fix is one `invlpg` per cleared entry, only when
the space is the installed one.

**The rest.**

- **`ARENA` held its leaf guard across allocation.** `Vec::push` in the grow
  path and `Vec::new` in `Drop` both call the allocator, rebuilding
  `ARENA → HEAP → FRAMES`, the order D-85 deleted, unrecorded. Growth is now
  reserved outside the lock; `Drop` sizes its buffer from `live` before taking
  it.
- **`insert`'s recovery path was a guaranteed no-op.** It called `close(0)`,
  and zero can never decode, so the slot it claimed to free stayed occupied.
- **`Mixer` gave table zero the identity.** The kernel derived it as
  `id * 0x9E3779B9`, zero for the first table, so the kernel's own handles were
  unmixed and its first was the literal integer three. The crate's doc claimed a
  small integer "is never a valid handle", false for any value with the reserved
  bits set. `Mixer::for_table` adds an odd constant after the multiply, and the
  sentence is corrected.
- **Rights were never checked against object type.** D-78's one rule is that a
  right names a distinction the kernel makes *about that object*. The first
  object shipped with `READ`, `WRITE`, `EXECUTE` and `MAP` granted on an event,
  which has no pages. The check named `rights are checked and only narrow` used
  that grant as its fixture. `Rights::are_meaningful_for` and a per-type mask
  make the rule enforceable; `insert` refuses the rest.

**Three checks could not fail.**

- *a handle does not cross tables* accepted any of three errors, and got `Stale`
  from the arena's bounds check. The mixers differ, so a foreign handle decodes
  outside the arena and never reaches the ownership comparison. **Deleting that
  comparison left the check green.** It now forges a handle with the other
  table's mixer, and breaking the comparison turns it red.
- The `x86-tables` layout test wrote a value into a zeroed array at the offset
  it was testing and read it back from the same place, an identity that passes
  under any layout. It now checks the transcribed offset against `offset_of!`.
- `a_malformed_value_never_decodes` skipped, with a `continue`, the values where
  its property was interesting.

**One break also behaved badly.** The first attempt at a check for the wait
window *hung the machine* instead of failing. Its predicate signalled the event,
which takes the queue lock the predicate already runs under. The second attempt
passed under the broken code, because `Event::wait`'s outer check short-circuits
and never reaches `wait_unless`. The third runs the wait on a spawned thread and
calls `wait_unless` directly, so a lost wakeup is a timeout in the *observer*,
red rather than hung.

**Rationale.** A fix is not exempt from the standard it was written to uphold.
D-94's TSS repair was meant to remove a torn store and introduced undefined
behaviour, under a `SAFETY` comment that argued the opposite. It was green,
checked, and wrong.

**The race a check steps around is the race nothing tests.** The existing event
check waits for the waiter to block before signalling. That was right for D-49's
reason and left the window itself unexercised. One CPU cannot drive it, which is
D-70 rule 6, so slice 9's model, not a self-check, will settle it.

**Consequences.**

- Ninety-six named machine checks. Six new or repaired ones, each broken.
- `011-userspace.md`'s lock-order section gains `ARENA`, which slice 6 added
  and recorded nowhere.
- **Two findings are carried, not fixed.** `undo` can still clear an entry that
  existed before the call, where a range overlaps an identical pre-existing
  mapping. Only the direct map's outward rounding produces that, and it aborts
  the whole space, but the next caller recovering from a `map_range` error will
  meet it. `NotYours` versus `Stale` is an occupancy oracle over a global arena:
  a table can learn which slots another holds. Zircon collapses both to one
  error for that reason. This is a decision for the phase that has a syscall to
  surface them through.
