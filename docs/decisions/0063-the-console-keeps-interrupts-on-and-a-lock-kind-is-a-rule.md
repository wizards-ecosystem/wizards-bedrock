# D-63 The console keeps interrupts on, and a lock kind is a rule

Status: Accepted, 2026-08-23.

**Context.** D-34 accepted that interrupt latency is bounded by the longest
critical section, named no bound, and declined a second lock type as "a type
parameter with one useful value still". This phase's charter required a
position: fix, or accept with the bound named.

**What the measurement said.** A console scroll on the gate's own machine takes
seventy milliseconds: a framebuffer moved and blitted under one lock, interrupts
off throughout. That is not a latency for any interrupt to wait behind, and
naming the bound would have meant accepting it.

**Decision.** `wost::sync::ThreadLock`: a spinning lock that does *not*
exclude interrupts and refuses to be taken from trap context. Both console
devices use it. The rule that makes it sound (nothing logs or draws from a
handler) was D-31's, in prose; it is now an assert against D-55's trap
depth. The keyboard handler's echo, and its `debug!`, moved to the reading
thread.

*Corrected 2026-08-23 by the phase 0 to 4 audit.* This said the handler's
`debug!` drew from a handler. It never did: `MAX_LEVEL` is `Info`, so
`record` returns before touching a device. The echo was the live violation.
The log line was latent, live once the level was raised. That argues better
for moving it, and the assert now removes the need to remember it.

**And a second rule, found by breaking the first.** Putting the console back
on a `SpinLock`, to see the new check go red, hung the machine. The log path
then took an interrupts-on lock inside an interrupts-off one, and the
preempted holder could never be scheduled. So `SpinLock` counts its holders,
and `ThreadLock` refuses to be taken while any is held. Two locks, each
correct alone, deadlocking together: a comment does not catch that.

**Rationale.** D-34's reasoning was right for its time, and its conclusion
has expired. There are two useful values now, and the second lets the
kernel's longest operation stop being everyone's latency floor.

**Consequences.**

- A `ThreadLock` holder can be preempted; a `SpinLock` holder cannot. That
  costs a waiter the rest of its slice and no more, since the timer takes
  the CPU back and gives it to the holder. The scheduler makes the trade
  safe, which is what D-34 said it was waiting for.
- The panic path is exempt from both asserts: the code it interrupted never
  resumes.
- The check is not how long a scroll takes but that interrupts keep
  arriving while it runs, counted inside the lock. Sampled outside, a
  pending expiry lands as the guard drops and the count moves either way.
  The first version did that, and passed under the break.
- The keystroke watch changed too. It kept the last press and release,
  which works only if a reader looks between keystrokes. Once the handler
  stopped logging, it decoded a whole injected burst in one drain, and a
  check waiting for the burst's first key waited out its ten seconds. It
  now keeps two accumulating bitmasks.
- Still owed: `SpinLock` holders are counted globally, one more entry on
  D-56's per-CPU list.
