# D-191 Process construction owns one rollback transaction until the thread is runnable

Status: Accepted, 2026-08-27. Amends D-100, D-106, D-115 and D-117; closes PW-103 and PW-109.

**Decision.** `ProcessConstruction` owns the private address space and the
exact owned ranges mapped into it. Its destructor unmaps those ranges, asks the
task table for release evidence and reclaims every table. The transaction
survives through `thread::spawn_in_space`; only its reserved-slot `arrange`
callback converts it into the `Bootstrap` and `Launch` records immediately
before the scheduler marks the thread ready. All channel, table and ring values
remain ordinary owned locals captured beside the transaction, so their normal
destructors cover every earlier refusal. The callback borrows rather than owns
the prepared transaction: if publication refuses, thread construction retires
the bound task first, and only then can the caller drop and reclaim its tables
with release evidence.

Thread construction exposes three machine-only refusal points after scheduler
reservation, task fabrication and address-space binding. Its arrangement
callback can also refuse publication before the slot becomes runnable. Each
rollback returns both scheduler and task state; the task-table retirement path
preserves the slot's immutable arena stack top while clearing its live state
and binding.

`load_image` now returns a `LoadedImage` containing its entry and exact
placements. The entry is checked during preflight and must lie within the
unrounded memory extent of an executable load segment before the first frame is
taken. There is no global last-load ownership record.

**Rationale.** `user::spawn` used `?` after creating an `AddressSpace`, which
deliberately has no automatic `Drop`, so every refusal abandoned at least the
root and often image and stack frames, keepalives, ring pages and table
ownership; the global `PLACED` record let a second construction replace
another's cleanup data; and `task::retire` erased the arena stack top
`fabricate` promises to reuse. One linear owner makes the commit boundary
visible in the type and makes ordinary `?` correct. Giving `AddressSpace` a
general destructor cannot remove owned leaf mappings it does not enumerate and
cannot prove a bound task released the root. Scattering cleanup across every
error arm would duplicate ownership transitions whose order must remain
explicit. Returning placements makes concurrent constructions independent by
value; a lock around global last-call state would serialize calls without
giving the right caller durable ownership.

**Alternatives.** Preallocating every resource before the address space would
still leave mapping and thread binding as fallible steps and would hold scarce
objects longer. Making the thread runnable and then asking it to self-destruct
on construction failure cannot reclaim the stack or tables it is running on.
Validating the entry only at the ring-3 crossing remains safe against execution
but is too late to make loading fail-atomic.

**Consequences.** `every refused spawn returns every resource` stops after each
of fourteen construction edges twice, warming retained allocator backing on the
first pass and requiring an exact second-pass baseline for physical frames,
live heap allocations, returned page/table tallies, handle slots and arena
commitment, channel ends, rings, scheduler slots and process records. It
includes reserved, fabricated and bound task states plus refused launch-record
publication. The loader check requires an invalid entry to refuse without
changing the frame count and keeps two successful loads' returned placement
records independent. Mutations 96 through 99 keep the transaction reclaim,
refused-arrangement scheduler release, entry preflight and stack-preserving
retirement red. Completed values commit directly into the reserved slot, with
no by-value return frame, because a nested `finish` return overflowed a guarded
debug kernel stack. No unsafe operation was added.
