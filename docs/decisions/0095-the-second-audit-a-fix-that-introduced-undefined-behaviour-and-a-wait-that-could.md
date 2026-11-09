# D-95 The second audit: a fix that introduced undefined behaviour, and a wait that could sleep for ever

Status: Accepted, 2026-08-24.

**Context.** A second adversarial pass, over slice 6 and over D-94's own
repairs. The baseline was green — 94 machine checks, every host test passing —
and it found eleven defects, three of them worse than anything in the first
pass.

**The worst was in a D-94 fix.** D-94 replaced a whole-`TaskState`
`write_volatile` with a targeted eight-byte store, arguing from the field's
width that the result was atomic. But `TaskState` is `#[repr(C, packed(4))]`
and `IST1` sits at offset `0x24` — four-mod-eight. In a segment on an
eight-byte boundary that field is *four*-byte aligned, so the store was
**misaligned, which is undefined behaviour**, and the atomicity the argument
rested on held by the linker's choice rather than by anything stated. The code
it replaced was correctly aligned and merely torn-observable. A fix made it
worse in the one dimension the fix was about.

The repair is to place the segment four-mod-eight, with four bytes of padding
in front, so that `0x24` lands on a boundary. `x86-tables` now pins both facts:
the offset is `0x24`, checked against `offset_of!` rather than against itself,
and it is four-mod-eight, which is *why* the padding exists.

**Second: `Event::wait` could sleep for ever, and three docs said it could
not.** The shape was check-then-register — read the condition, then call into
the wait queue, which registers and blocks. A signal landing in the gap woke an
empty queue, and the waiter then blocked with nothing left to wake it. The
claim that a sticky signal covered this was wrong in a way worth naming: the
state can only be re-read once a block *returns*, and it returns only on a
wake, so stickiness rescues the one case that stops arising as soon as an idle
thread exists.

`WaitQueue::wait_unless` replaces it. Registration and the re-check happen
under the same queue lock a waker must take, so the two are one step and a
signal cannot land between them. It also clears the registration bit on the way
out, which the old path never did on a non-wake return — a stale bit names a
thread that is no longer waiting there.

**Third: D-94's rollback cleared live page-table entries without invalidating
them.** `undo` runs against the *active* space, and `map_owned` hands the
frames straight back to the allocator afterwards — the same stale-TLB hazard
D-94 fixed for `unmap`, arriving by the other door. One `invlpg` per cleared
entry, and only when the space is the installed one.

**The rest.**

- **`ARENA` held its leaf guard across allocation.** `Vec::push` in the grow
  path and `Vec::new` in `Drop` both call the allocator, rebuilding
  `ARENA → HEAP → FRAMES` — the order D-85 deleted, reintroduced without being
  written down. Growth is now reserved outside the lock; `Drop` sizes its
  buffer from `live` before taking it.
- **`insert`'s recovery path was a guaranteed no-op.** It called `close(0)`,
  and zero can never decode, so the slot it claimed to free stayed occupied.
- **`Mixer` gave table zero the identity.** The kernel derived it as
  `id * 0x9E3779B9`, which is zero for the first table — so the kernel's own
  handles were unmixed and its first one was the literal integer three. And
  the crate's doc claimed a small integer "is never a valid handle", which is
  false for any value with the reserved bits set. `Mixer::for_table` adds an
  odd constant after the multiply; the sentence is corrected.
- **Rights were never checked against object type.** D-78's one rule is that a
  right names a distinction the kernel makes *about that object*, and the first
  object shipped with `READ`, `WRITE`, `EXECUTE` and `MAP` granted on an event,
  which has no pages. Worse, the check named `rights are checked and only
  narrow` used that grant as its fixture. `Rights::are_meaningful_for` and a
  per-type mask make the rule enforceable; `insert` refuses the rest.

**Three checks could not fail.**

- *a handle does not cross tables* accepted any of three errors, and the one it
  actually got was `Stale` from the arena's bounds check — because two mixers
  differ, so a foreign handle decodes to an index that is not in the arena and
  never reaches the ownership comparison. **Deleting that comparison left the
  check green.** It now forges a handle with the other table's mixer, and
  breaking the comparison turns it red.
- The `x86-tables` layout test wrote a value into a zeroed array at the offset
  it was testing and read it back from the same place — an identity that would
  pass under any layout. It now checks the transcribed offset against
  `offset_of!`.
- `a_malformed_value_never_decodes` skipped, with a `continue`, exactly the
  values where its property was interesting.

**And one break behaved badly.** The first attempt at a check for the wait
window *hung the machine* instead of failing: its predicate signalled the
event, which takes the queue lock the predicate already runs under. The second
attempt passed under the broken code, because `Event::wait`'s outer check
short-circuits and never reaches `wait_unless`. The third runs the wait on a
spawned thread and calls `wait_unless` directly, so a lost wakeup is a timeout
in the *observer* — red rather than hung. Three attempts to make one check able
to fail honestly.

**Rationale.** Two things this pass says that the first did not.

The first is that **a fix is not exempt from the standard it was written to
uphold**. D-94's TSS repair was written to remove a torn store and introduced
undefined behaviour, in a `SAFETY` comment that argued for the opposite. It was
green, checked, and wrong.

The second is that **the race a check steps around is the race nothing tests**.
The existing event check waits for the waiter to have blocked before
signalling, which was right for D-49's reason and left the actual window
unexercised. One CPU cannot drive it, and that is D-70 rule 6's whole point —
which is why slice 9's model, not a self-check, is what will settle it.

**Consequences.**

- Ninety-six named machine checks. Six new or repaired ones, each broken.
- `011-userspace.md`'s lock-order section gains `ARENA`, which slice 6 added
  and recorded nowhere.
- **Two findings are carried, not fixed.** `undo` can still clear an entry that
  existed before the call, where a range overlaps an identical pre-existing
  mapping — only the direct map's outward rounding produces that, and a failure
  there aborts the whole space, but the next caller that recovers from a
  `map_range` error will meet it. And `NotYours` versus `Stale` is an occupancy
  oracle over a global arena: a table can learn which slots another holds.
  Zircon collapses both to one error for that reason, and this is a decision
  for the phase that has a syscall to surface them through.
