# D-167 A dead program's work is finished, not forgotten

Status: Accepted (2026-08-25)

**Context.** D-153 said an operation in flight becomes one more thing the
zombie holds, and `012` §10 added the part the original entry did not say:
**exit must be a wait, not a signal.** io_uring cancelled out of line for six
years and the symptom was mount points staying busy after the process had
exited.

Until slice 1 there was nothing to say this about. Every operation this kernel
served completed inside the call that submitted it, so a retiring program had
no work in flight by construction. `Opcode::Timer` changed that, and D-153's
debt became reachable in the same commit that made it possible.

**The hazard is specific.** A retired program's ring pages go back to the
allocator in `reclaim`. An operation still in flight is one that will later
write a completion, and the page it would write into is about to be somebody
else's. That is not a leak; it is a write to freed memory, arriving at whatever
time the operation happens to finish.

**Decision.** The ring is quietened before anything it could touch is released.

`Queue::abandon` ends every operation in flight and **posts the completion it
was owed**, into pages the zombie still holds. When it returns, nothing is left
that could reach them, and `reclaim` drops the ring immediately after.

Not "marked abandoned and left". The completions go into a ring nobody will
read, and that is not waste: posting them is what *spends the reservations*.
A reservation never spent is a ring whose accounting says work is outstanding
for ever, which is invisible until something asks — and slice 5's device will
ask.

**No grace period and no refcount.** `012` §10 names the third consequence and
it is the easiest to violate by accident: a teardown that waits for a count to
reach zero "eventually" is exactly the out-of-line cancellation that took
io_uring six years to undo. Every operation this kernel has today ends
synchronously, so one sweep is the whole wait. An operation that could not — a
read the device has already been told to do — is slice 5's, and the shape it
must fit is this one: the sweep ends when `is_quiet` is true and not before.

**The handle promise needs no code at all**, which is the part worth recording.
D-151's third terminal path is "a retired program's", and D-163 made the
reservation a value holding an `Arc` to its table. So a program that dies
holding a promise leaves the table alive exactly as long as the promise does,
and the promise's destructor gives the slot back and lets the table go. The
teardown drops its reference and the table simply does not leave. "The zombie
holds every resource any in-flight operation can still touch" is true by
construction rather than by remembering, which is what D-163 predicted and this
slice is the confirmation of.

**Rationale.** Three checks, and each is a different claim.

*Work in flight is ended before the ring is released* asserts the completions
arrive, say `CANCELLED` rather than pretending to be results, and — the part
that matters — that `outstanding` returns to zero. It also sweeps twice, so
teardown is idempotent: `reclaim` is not the only path that can reach a zombie.

*A promise outlives the program that made it* observes the table through a
`Weak`, which is the only way to ask "is it still there" without being the
reason it is, and then **uses** it: the promise is spent against the orphaned
table, because surviving and working are different claims. Then the `Weak` must
stop upgrading, because a table nothing will ever free is the other failure.

**Consequences.**

- `Queue::abandon` and `Queue::is_quiet`; `Reclaimed` gains `abandoned`.
- Two machine checks, three breaks, all red. Breaking the promise's `Arc` —
  giving the reservation a table of its own — reddens four checks at once,
  which is the redundancy D-165 argued for doing its job.
- The ordering in `reclaim` is now load-bearing and says so: quieten, then drop
  the table, then drop the ring, then release the space.
- Slice 5 inherits the shape rather than the code. A polled NVMe read cannot be
  ended by writing a completion — the device has the command — so `abandon`
  will have to wait for the controller there. What it must not do is return
  before `is_quiet`.
