# D-153 An operation in flight is collected with its program's zombie

Status: Accepted, 2026-08-25.

**Decision.** An operation whose program retires before it completes is
collected with the program's zombie (D-117): the ring outlives the program until
whoever collects the zombie collects it too. The poster does not drop the
completion, and submission is not refused once the program is retiring. A
completion landing for an already-collected slot is the bug this rules out.

**Rationale.** It is the machinery that already exists: `LastRites`, `orphan`
and `collect` were built by D-117 for the program's memory, and
`Bootstrap::ring` is already an `Arc` (D-126). Dropping at post needs the poster
to check the slot's generation, reinventing the check handles are
generation-tagged for, and getting it wrong posts into a re-used slot. Refusing
at submit loses a race the submitter cannot see, because retirement is decided
elsewhere. The cost is that a zombie can outlive its program by an operation
rather than by a scheduling quantum, which is the honest cost of asynchrony.

**Consequences.**

- `LastRites` grows what the ring was holding. `collect` releases the
  reservations D-151 took, handle-table slots included, which is the third
  terminal path that entry left owed.
- A completion posted to a zombie's ring is written and never read. That is
  correct: exactly one terminal completion is still posted for every operation
  (D-79), and nobody is waiting for it.
- The check is that frames, handles and reservations all come back after a
  program retires with an operation outstanding, a state the existing
  reclamation checks do not reach.

**Amended by `012` §10: exit must be a wait, not a signal.** Closing an
io_uring instance cancels pending requests automatically — and until March 2025
those cancellations happened *out of line*, so applications could not rely on
ring exit meaning the I/O had finished. The symptom in Axboe's cover letter was
**mount points staying busy after the test process had exited**. Six years of
io_uring did it the other way and it was a bug.

Three consequences for this entry, none of which it originally said:

- **Collection must wait for the operation, not merely mark it abandoned.**
- **The zombie must hold every resource any in-flight operation can still
  touch.** The busy-mount symptom is what happens when it does not, and it is
  the same failure as a completion posted into a re-used slot.
- **Do not put a grace-period-based refcount on the teardown path.** The same
  fix replaced a `percpu_ref` with a plain atomic because the RCU grace period
  was too slow there, and this kernel's reclamation is already deferred to
  whoever has nothing better to do (D-117).

**Left owed.** Cancellation is not this. An operation whose program retires is
abandoned; an operation a *living* program wants to stop needs the advisory
cancel D-79 describes, and nothing here builds it. `012` §8 adds what that will
need: two completions, order unspecified, and `-EALREADY` meaning "your
completion is coming, just not because of you".
