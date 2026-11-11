# D-63 The console keeps interrupts on, and a lock kind is a rule

Status: Accepted, 2026-08-23.

**Decision.** `wost::sync::ThreadLock`: a spinning lock that does *not*
exclude interrupts and refuses to be taken from trap context. Both console
devices use it. The rule that makes it sound (nothing logs or draws from a
handler) was D-31's, in prose; it is now an assert against D-55's trap
depth. The keyboard handler's echo, and its `debug!`, moved to the reading
thread.

*Corrected 2026-08-23.* The handler's `debug!` never drew: `MAX_LEVEL` is
`Info`, so `record` returns before touching a device. The echo was the live
violation; the log line was latent, live once the level was raised.

**A second rule.** An interrupts-on lock taken inside an interrupts-off one
deadlocks, because the preempted holder can never be scheduled. So
`SpinLock` counts its holders, and `ThreadLock` refuses to be taken while
any is held. Two locks, each correct alone, deadlocking together: a comment
does not catch that.

**Rationale.** D-34 bounded interrupt latency by the longest critical
section and named no bound. A console scroll held one lock for about seventy
milliseconds with interrupts off, which is not a latency for any interrupt
to wait behind. With a scheduler there are now two useful lock kinds, and
the second lets the kernel's longest operation stop being everyone's latency
floor.

**Consequences.**

- A `ThreadLock` holder can be preempted; a `SpinLock` holder cannot. That
  costs a waiter the rest of its slice and no more, since the timer takes
  the CPU back and gives it to the holder.
- The panic path is exempt from both asserts: the code it interrupted never
  resumes.
- Still owed: `SpinLock` holders are counted globally, one more entry on
  D-56's per-CPU list.
