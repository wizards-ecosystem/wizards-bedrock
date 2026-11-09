# D-63 The console keeps interrupts on, and a lock kind is a rule

Status: Accepted, 2026-08-23.

**Context.** D-34 accepted that interrupt latency is bounded by the longest
critical section, named no bound, and declined a second lock type as "a type
parameter with one useful value still". This phase's charter required a
position: fix, or accept with the bound named.

**What the measurement said.** A console scroll on the gate's own machine is
seventy milliseconds — a framebuffer moved and blitted under one lock, with
interrupts off for all of it. That is not a latency for any interrupt to wait
behind, and naming it would have been accepting it.

**Decision.** `wost::sync::ThreadLock`: a spinning lock that does *not*
exclude interrupts and refuses to be taken from trap context. Both console
devices use it. The rule that makes it sound — nothing logs or draws from a
handler — was already D-31's, stated in prose; it is now an assert against
D-55's trap depth. The keyboard handler's echo, and its `debug!`, moved to the
reading thread.

*Corrected 2026-08-23 by the phase 0–4 audit.* This said the handler's
`debug!` drew from a handler. It never did: `MAX_LEVEL` is `Info`, so `record`
returns before touching a device. The echo was the live violation; the log
line was a latent one that would have become real the moment the level was
raised — which is the better argument for moving it, and the one the assert
now makes unnecessary to remember.

**And a second rule, found by breaking the first.** Putting the console back
on a `SpinLock` to check that the new check went red made the machine hang
instead: the log path then took an interrupts-on lock from inside an
interrupts-off one, and the preempted holder could never be scheduled. So
`SpinLock` counts its holders and `ThreadLock` refuses to be taken while any
is held. Two locks each individually correct, deadlocking together, is
exactly the kind of thing a comment does not catch.

**Rationale.** D-34's reasoning was right for its time and its conclusion has
expired: there are two useful values now, and the second one is what lets the
longest operation in the kernel stop being everyone's latency floor.

**Consequences.**

- A `ThreadLock` holder can be preempted, which a `SpinLock` holder cannot.
  That costs a waiter the rest of its slice and no more, because the timer
  takes the CPU back and gives it to the holder — the scheduler is what makes
  the trade safe, which is precisely what D-34 said it was waiting for.
- The panic path is exempt from both asserts: the code it interrupted never
  resumes.
- The check is not how long a scroll takes but that interrupts keep arriving
  while it runs, counted from inside the lock. Sampled outside it, a pending
  expiry lands the instant the guard drops and the count moves either way —
  which the first version did, and it passed under the break.
- The keystroke watch had to change with it. It kept the last press and the
  last release, which works only if a reader looks between one keystroke and
  the next; once the handler stopped logging it became fast enough to decode
  a whole injected burst in one drain, and a check waiting for the first key
  of that burst waited out its ten seconds. Two accumulating bitmasks now.
- Still owed: `SpinLock` holders are counted globally, which is one more
  entry on D-56's per-CPU list.
