# D-93 Waking a thread from thread context owes it a scheduling decision

Status: Accepted, 2026-08-24.

**Decision.** `unblock` arms the timer for now when it wakes anything from
thread context. From a handler it does not: the trap exit is about to make the
decision, and arming twice is noise. This keeps the rule `thread.rs` states:
*"Every path that changes who is runnable ends by arming the timer for the next
moment a decision is due, or stopping it when there is none (D-57)."* Without
it, a thread woken from thread context stayed Ready until an unrelated
interrupt arrived: unbounded, on a tickless kernel with nothing armed.

**Rationale.** Making the waker yield is stronger and wrong. Signalling an event
is not a scheduling point the caller asked for, and a waker that yields cannot
signal two things without being descheduled between them. Arming says "a
decision is due" and leaves the policy to the scheduler, where D-53 put it.

**Alternatives.** Yield in the waker (rejected above). Arm from trap context too
(rejected: redundant, and the trap path already computes the *right* deadline
rather than "now"). Require callers to reschedule (rejected: the primitive
should keep the rule its module states).

**Consequences.**

- The check *a signal wakes a thread waiting on a handle* waits for the waiter
  to block before signalling, so it cannot pass on a thread that never slept
  (D-49's lesson), and its break removes this arm.
- Every future object that signals from thread context inherits the fix.
