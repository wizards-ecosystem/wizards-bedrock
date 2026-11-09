# D-93 Waking a thread from thread context owes it a scheduling decision

Status: Accepted, 2026-08-24.

**Context.** `kernel/src/thread.rs`'s module doc states the rule the tickless
design rests on: *"Every path that changes who is runnable ends by arming the
timer for the next moment a decision is due, or stopping it when there is
none (D-57)."* `unblock` changes who is runnable and armed nothing.

Phase 4 never noticed, because the only caller was a wait queue woken from an
interrupt handler — where the trap exit reschedules anyway (D-54). Phase 5's
first kernel object woke a thread from **thread context**, and the woken thread
sat Ready for as long as it took an unrelated interrupt to arrive. On a
tickless kernel with nothing armed, that is unbounded.

**Decision.** `unblock` arms the timer for now when it wakes anything from
thread context. From a handler it does not: the trap exit is about to make the
decision, and arming twice is noise.

**Rationale.** The alternative — making the waker yield — is stronger and
wrong: signalling an event is not a scheduling point the caller asked for, and
a waker that yields cannot signal two things without being descheduled between
them. Arming says "a decision is due" and leaves the policy to the scheduler,
which is where D-53 put it.

The bug was found by a self-check that timed out rather than by reading, and
the check only existed because a new object needed one. Worth recording: the
first caller of an old primitive from a new context is where its unstated
assumptions surface.

**Alternatives.** Yield in the waker (rejected above); arm from trap context
too (rejected: redundant, and the trap path already computes the *right*
deadline rather than "now"); require callers to reschedule (rejected: it is the
rule this module already states, so the primitive should keep it).

**Consequences.**

- `Event::signal` from thread context now wakes promptly. The check that found
  this — *a signal wakes a thread waiting on a handle* — waits for the
  waiter to have actually blocked before signalling, so it cannot pass on a
  thread that never slept (D-49's lesson), and its break is to remove this arm.
- Every future object that signals from thread context inherits the fix rather
  than rediscovering it.
