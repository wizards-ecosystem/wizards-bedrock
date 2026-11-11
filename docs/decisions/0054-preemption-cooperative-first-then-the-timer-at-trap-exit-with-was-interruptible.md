# D-54 Preemption: cooperative first, then the timer at trap exit, with "was interruptible" as the test

Status: Accepted, 2026-08-22. Amended by D-76.

**Decision.** The tick handler dismisses the interrupt, then calls
`thread::reschedule` with the trap frame. It switches only if the frame's
interrupt flag was set. That test is free and exact. D-34 clears the flag
while any lock is held, and the scheduler clears it from decision to switch.
So every critical section is unpreemptible without a preempt count, and
nothing is switched out mid-decision. `wost::task::switch_from_trap` makes
the frame the evidence: it refuses a frame whose flag is clear, and refuses
to be called outside a handler.

The handler's frame stays on the preempted thread's stack; the switch saves
the path down to it. When next picked, the thread resumes inside its own
handler, returns through the trampoline, and `iretq` restores the flags the
CPU pushed.

Asterinas's `PreemptDisabled` guard discipline, which D-34 left for "once
there is a scheduler", is **declined**. With the flag as the test, a lock
*is* a preemption guard; a second lock type would still be a type parameter
with one useful value.

**Rationale.** A deferred-reschedule flag would be one more state to keep
consistent. End-of-interrupt must come before the switch: the APIC holds the
timer's priority class until the EOI, so a thread running inside it would
hold every later tick hostage.

**Consequences.** A thread preempted inside a handler and one that yielded
resume by different paths, and neither needs to know which: the switch
ignores what is above it on the stack.
