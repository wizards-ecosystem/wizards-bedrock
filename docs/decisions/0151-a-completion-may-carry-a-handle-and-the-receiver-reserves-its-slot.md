# D-151 A completion may carry a handle, and the receiver reserves its slot

Status: Accepted (2026-08-25) (amended by D-181), amends D-111's provenance clause

**Context.** `011` closed Phase 5 owing this: *"no completion carries a handle,
so the ring cannot be the delegation path the channels are."* Phase 6 makes it
concrete. Duplicate became the first implementation and proof; OpenAt was the
motivating namespace caller whose useful result is newly derived authority.

The question is who accounts for the handle-table slot.

**Options.**

1. **The kernel installs it** at completion time. Simplest: the operation
   finishes, the kernel puts the handle in the receiver's table, the completion
   names it.
2. **The receiver reserves it at submission**, from the same quota and by the
   same admission the completion slot already uses.
3. **Defer** — the ring stays data-only and every authority-producing operation
   detours through a channel.

**Decision.** Option 2. A submission that may produce a handle reserves a
handle-table slot alongside its completion slot, and is refused at admission if
either is unavailable.

**Rationale.** Option 1 reintroduces exactly the failure the reservation model
exists to delete. D-79 chose RIO-style admission over io_uring's `CQ_NODROP`
so that **an unreportable operation is impossible rather than unlikely**; a
handle installed at completion time can fail for a reason the submitter was
never told about, and the ring then has two flow-control models — one for the
completion, none for the authority it carries.

**The io_uring evidence, corrected.** An earlier draft of this entry said
io_uring grew "direct descriptors — `IORING_OP_ACCEPT_DIRECT`" *because*
completion-time installation exhausted the descriptor table. A research pass
checked it against the primary record and both halves were wrong.

There is **no `IORING_OP_ACCEPT_DIRECT` opcode.** It is `IORING_OP_ACCEPT`
with `sqe->file_index` set; `io_uring_prep_accept_direct()` is a liburing
helper, not an ABI entry. And the merge argument was never exhaustion: Begunkov's
5.15 cover letter, the `IORING_FILE_INDEX_ALLOC` commit message and
`io_uring_registered_files(7)` all give **per-operation overhead and
round-trips** — collapsing accept, register-update and close into one
operation. None of them mentions backpressure or table limits.

**The conclusion survives, on a better argument.** io_uring can install at
completion time only because it has an escape this project has explicitly
refused: `IORING_FEAT_NODROP` gives it an **unbounded kernel-side overflow
backlog** for completions that will not fit. D-79 rejected exactly that in
favour of RIO-style admission. Having refused the backlog, completion-time
installation has nowhere to put a failure — so reservation is not a
preference here, it is what the earlier decision already implies.

Two observable symptoms in io_uring's own source support the reading.
`io_accept_prep()` refuses multishot combined with a *specific* fixed slot,
because one submission producing unbounded installs is the case that forces
kernel-managed allocation. And `IORING_FILE_INDEX_ALLOC` still does not
reserve at submission: it allocates at completion and reports `-ENFILE` in
`cqe->res` when the range is full — a resource failure delivered at the one
moment the submitter can no longer do anything about it.

Zircon is the shape this entry chose, from the start: a channel read takes a
caller-supplied handle array, so the space is committed before the message
moves.

Option 3's cost is two mechanisms and two round trips for one idea, and it
does not avoid the decision — it moves it to whoever writes `open`.

**Consequences.**

- A submission carries what it may produce. The opcode says whether a handle
  can come back, and admission checks the handle quota then, not later.
- Refusal is backpressure and reads like it: the program is told at submit,
  which is the moment it can do something about it.
- For same-object Duplicate, completion-carried rights attenuate the way D-110
  requires. A cross-type producer may derive only a type-specific right
  authorised by its source operation and preserve only universal rights already
  held; OpenAt's `TRAVERSE`-to-`READ` rule is D-173's worked case, repaired by
  D-181.
- Quota accounting moves earlier and has to be released if the operation is
  cancelled or its program retires (D-153).

**Left owed.** The reservation must be *returned* on every terminal path — a
refused operation, a cancelled one, a retired program's. That is three paths
and the third is D-153's, so this entry does not close it.
