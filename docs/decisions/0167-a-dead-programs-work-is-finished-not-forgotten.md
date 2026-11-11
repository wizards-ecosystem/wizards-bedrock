# D-167 A dead program's work is finished, not forgotten

Status: Accepted, 2026-08-25.

**Decision.** The ring is quietened before anything it could touch is released.
`Opcode::Timer` made D-153's debt reachable: a retired program's ring pages go
back to the allocator in `reclaim`, and an operation still in flight would later
write its completion into somebody else's page. `Queue::abandon` ends every
operation in flight and **posts the completion it was owed**, saying `CANCELLED`
rather than pretending to be a result, into pages the zombie still holds. When
it returns nothing can reach them, and `reclaim` drops the ring immediately
after. The order in `reclaim` is load-bearing and says so: quieten, then drop
the table, then drop the ring, then release the space.

The completions are posted, not marked abandoned, because posting them is what
spends the reservations; a reservation never spent is a ring whose accounting
says work is outstanding for ever. **No grace period and no refcount** (`012`
§10): a teardown that waits for a count to reach zero "eventually" is the
out-of-line cancellation io_uring took six years to undo. Every operation today
ends synchronously, so one sweep is the whole wait. An operation that cannot,
such as a read the device has already been told to do, must fit this shape: the
sweep ends when `is_quiet` is true and not before.

**The handle promise needs no code.** D-151's third terminal path is "a retired
program's", and D-163 made the reservation a value holding an `Arc` to its
table, so the table lives exactly as long as the promise and the promise's
destructor returns the slot and lets the table go. "The zombie holds every
resource any in-flight operation can still touch" is true by construction
rather than by remembering, as D-163 predicted.

**Rationale.** `012` §10: exit must be a wait, not a signal. An operation in
flight at reclaim is not a leak but a write to freed memory, arriving whenever
the operation happens to finish.

**Consequences.**

- `Queue::abandon` and `Queue::is_quiet`; `Reclaimed` gains `abandoned`.
- The teardown check asserts the completions arrive, say `CANCELLED`, and that
  `outstanding` returns to zero, and sweeps twice, because `reclaim` is not the
  only path that can reach a zombie.
- The promise check observes the orphaned table through a `Weak`, the only way
  to ask whether it is still there without being the reason it is, spends the
  promise against it because surviving and working are different claims, and
  then requires the `Weak` to stop upgrading, because a table nothing will ever
  free is the other failure. Giving the reservation a table of its own reddens
  four checks at once, the redundancy D-165 argued for doing its job.
- A polled NVMe read cannot be ended by writing a completion, because the
  device has the command, so `abandon` will have to wait for the controller
  there. What it must not do is return before `is_quiet`.
