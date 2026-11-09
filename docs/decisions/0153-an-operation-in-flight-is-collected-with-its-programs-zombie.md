# D-153 An operation in flight is collected with its program's zombie

Status: Accepted (2026-08-25)

**Context.** A program submits an operation and retires before it completes.
The completion has somewhere to go only while the program's ring exists, and
D-117 already makes a retired program's resources a zombie that somebody
collects later. What happens to the operation was unspecified, and it touches
D-117 directly: **a completion landing for an already-collected slot is the
shape of the bug.**

**Options.**

1. **Dropped at post.** The poster checks whether the program is still there
   and discards the completion if not.
2. **Refused at submit** once the program is retiring, so nothing is in flight
   when it goes.
3. **Collected with the zombie.** The ring outlives the program until whoever
   collects the zombie collects it too.

**Decision.** Option 3.

**Rationale.** It is the machinery that already exists. `LastRites`, `orphan`
and `collect` were built by D-117 for the program's *memory*; an operation in
flight is one more thing the zombie holds until a thread with nothing better to
do picks it up. `Bootstrap::ring` is already an `Arc` (D-126), so the ring
already outlives the moment the program stops running.

Option 1 needs the poster to know the slot's **generation** — not just that a
program is there, but that it is the same one — which is the check every other
use-after-free in this kernel has needed and the reason handles are
generation-tagged at all. Getting it right means reinventing that here; getting
it wrong means posting into a re-used slot.

Option 2 needs the submitter to lose a race it cannot see: retirement is
decided elsewhere, and "is it retiring" is true a moment after it is asked.

The cost is that reclamation is not complete until in-flight operations have
finished or been abandoned, so a zombie can outlive its program by an operation
rather than by a scheduling quantum. That is the honest cost of the thing being
asynchronous.

**Consequences.**

- `LastRites` grows what the ring was holding. `collect` releases the
  reservations D-151 took — handle-table slots included — which is the third
  terminal path that entry left owed.
- A completion posted to a zombie's ring is written and never read. That is
  correct: exactly one terminal completion is still posted for every operation
  (D-79), and nobody is waiting for it.
- The check is that frames, handles and reservations all come back after a
  program retires *with an operation outstanding* — which is a state the
  existing reclamation checks do not reach.

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
