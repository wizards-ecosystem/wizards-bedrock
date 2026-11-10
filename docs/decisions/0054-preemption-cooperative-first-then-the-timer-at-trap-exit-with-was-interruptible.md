# D-54 Preemption: cooperative first, then the timer at trap exit, with "was interruptible" as the test

Status: Accepted, 2026-08-22. Amended by D-76.

**Context.** Slice 1 proved the switch under yields alone. Something had to
take the CPU from a thread that never gives it up: where may that switch
happen, and what must stop it?

**Options.** Preemptive from the first commit, coupling the two hardest new
mechanisms in one change. A deferred-reschedule flag acted on at a later
safe point, one more state to keep consistent. A switch from inside the
timer's handler, gated on whether the interrupted code was interruptible.

**Decision.** The tick handler dismisses the interrupt, then calls
`thread::reschedule` with the trap frame. It switches only if the frame's
interrupt flag was set. That test is free and exact. D-34 clears the flag
while any lock is held, and the scheduler clears it from decision to switch.
So every critical section is unpreemptible without a preempt count, and
nothing is switched out mid-decision. `wost::task::switch_from_trap` makes
the frame the evidence: it refuses a frame whose flag is clear, and refuses
to be called outside a handler.

The handler's frame stays on the preempted thread's stack; the switch saves
the path down to it. When next picked, the thread resumes inside
its own handler, returns through the trampoline, and `iretq` restores the
flags the CPU pushed.

Asterinas's `PreemptDisabled` guard discipline, which D-34 left for "once
there is a scheduler", is **declined**. With the flag as the test, a lock
*is* a preemption guard; a second lock type would still be a type parameter
with one useful value.

**Rationale.** End-of-interrupt must come before the switch. The APIC holds
the timer's priority class until the EOI, so a thread running inside it
would hold every later tick hostage. Cooperative first was the right order:
all of slice 1's checks ran unchanged under slice 2's preemption, which
proving the switch in isolation bought.

**Consequences.**

- A thread preempted inside a handler and one that yielded resume by
  different paths, and neither needs to know which: the switch ignores what
  is above it on the stack.
- Two checks: a spinner cannot starve the boot thread, which also spins
  (red with preemption disabled), and runtime divides 1:2 by weight within
  coarse bounds (red with the weight ignored).
- One hundred ticks a second now each cost a lock and a pick. Slice 3
  retires the tick and the cost with it.
