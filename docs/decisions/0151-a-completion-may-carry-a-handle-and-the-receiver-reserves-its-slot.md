# D-151 A completion may carry a handle, and the receiver reserves its slot

Status: Accepted, 2026-08-25. Amended by D-181. Amends D-111's provenance clause.

**Decision.** A completion may carry a handle. A submission that may produce a
handle reserves a handle-table slot alongside its completion slot, from the same
quota and by the same admission, and is refused at admission if either is
unavailable. The kernel does not install the handle at completion time, and the
ring does not stay data-only with authority detouring through a channel.
Duplicate is the first implementation; OpenAt is the motivating namespace
caller whose result is newly derived authority.

**Rationale.** D-79 chose RIO-style admission over io_uring's `CQ_NODROP` so
that an unreportable operation is impossible rather than unlikely. A handle
installed at completion time can fail for a reason the submitter was never told
about, and the ring would have two flow-control models: one for the completion,
none for the authority it carries. io_uring can install at completion only
because `IORING_FEAT_NODROP` gives it an unbounded kernel-side overflow backlog,
which D-79 refused; without that backlog a late install has nowhere to put a
failure, so reservation is what D-79 already implies. Zircon has the same shape:
a channel read takes a caller-supplied handle array, so the space is committed
before the message moves. A data-only ring costs two mechanisms and two round
trips for one idea, and only moves the decision to whoever writes `open`.

**Corrected.** An earlier draft said io_uring grew `IORING_OP_ACCEPT_DIRECT`
because completion-time installation exhausted the descriptor table. Both halves
were wrong. There is no such opcode: it is `IORING_OP_ACCEPT` with
`sqe->file_index` set, and `io_uring_prep_accept_direct()` is a liburing helper.
Begunkov's 5.15 cover letter, the `IORING_FILE_INDEX_ALLOC` commit and
`io_uring_registered_files(7)` give per-operation overhead and round trips as
the reason, not exhaustion. `IORING_FILE_INDEX_ALLOC` still allocates at
completion and reports `-ENFILE` in `cqe->res` when the range is full: a
resource failure at the one moment the submitter can no longer act on it.

**Consequences.**

- The opcode says whether a handle can come back, and admission checks the
  handle quota then, not later.
- Refusal is backpressure: the program is told at submit, the moment it can do
  something about it.
- For same-object Duplicate, completion-carried rights attenuate the way D-110
  requires. A cross-type producer may derive only a type-specific right
  authorised by its source operation and preserve only universal rights already
  held; OpenAt's `TRAVERSE`-to-`READ` rule is D-173's worked case, repaired by
  D-181.
- The reservation must be returned on every terminal path: a refused operation,
  a cancelled one, and a retired program's. The third is D-153's, so this entry
  does not close it.
