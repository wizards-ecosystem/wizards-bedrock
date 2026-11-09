# D-54 Preemption: cooperative first, then the timer at trap exit, with "was interruptible" as the test

Status: Accepted (2026-08-22) (amended by D-76)

**Context.** Slice 1 proved the switch under yields alone. Something had to
take the CPU from a thread that never gives it up, and the question was
where that switch may happen and what must stop it.

**Options.** Preemptive from the first commit (couples the two hardest new
mechanisms in one change); a deferred-reschedule flag acted on at some later
safe point (another state to keep consistent); a switch from inside the
timer's handler, gated on whether the interrupted code could be interrupted.

**Decision.** The tick handler dismisses the interrupt and then calls
`thread::reschedule` with the trap frame. It switches only if the frame's
interrupt flag was set. That test is free and exact: D-34 clears the flag
while any lock is held, and the scheduler itself clears it from decision to
switch, so every critical section in the kernel is unpreemptible without a
preempt count, and nothing is ever switched out of the middle of a decision.
`wost::task::switch_from_trap` makes the frame the evidence — it refuses a
frame whose flag is clear, and refuses to be called outside a handler.

The handler's own frame stays on the preempted thread's stack; what the
switch saves is the handler's path down to it. When the thread is next picked
it resumes inside its own handler, returns through the trampoline, and
`iretq` restores the flags the CPU pushed.

Asterinas's `PreemptDisabled` guard discipline, which D-34 left for "once
there is a scheduler", is **declined**: with the flag as the test, a lock
*is* a preemption guard, and a second lock type would be a type parameter
with one useful value still.

**Rationale.** The ordering — end-of-interrupt before the switch — is not a
detail: the APIC holds the timer's priority class until the EOI, and a thread
running inside it would hold every later tick hostage. Cooperative first was
the right sequencing: every one of slice 1's checks ran under slice 2's
preemption unchanged, which is what proving the switch in isolation bought.

**Consequences.**

- A thread that was preempted inside a handler and one that yielded resume
  by different paths, and both work without either knowing which it is; the
  switch does not care what is above it on the stack.
- Two checks: a spinner cannot starve the boot thread, which also spins
  (red with preemption disabled), and runtime divides 1:2 by weight within
  coarse bounds (red with the weight ignored).
- One hundred ticks a second now each cost a lock and a pick. Slice 3
  retires the tick and the cost with it.
