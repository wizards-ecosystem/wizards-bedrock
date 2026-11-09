# D-58 Sleeping: a deadline, a preserved lag, and a halt for the degenerate case

Status: Accepted, 2026-08-23.

**Context.** Tickless is only worth building if something wants to wait. D-53
built lag-clamped placement — a sleeper keeps its debt and banks at most one
slice of credit — and noted that nothing could sleep yet, so the clamp had no
caller. This is the caller.

**Decision.** A thread state, a deadline, and a saved lag. `sleep_ms` charges
the thread, records the lag it is leaving with, marks it sleeping until an
absolute deadline, and hands the CPU to whoever the policy picks. Every
scheduling path wakes whatever is due before it decides anything, and a woken
thread is placed back at the queue's fair-share point offset by the lag it
left with — which is where D-53's clamp acts, at last against a real sleeper.

The wait is a loop, not a single switch: a thread can be woken by something
other than its own deadline, and the deadline is what was promised.

**The degenerate case is deliberate.** A thread that sleeps with nothing else
runnable keeps the CPU and halts on it — still the running thread, still the
one the table charges, waiting on the timer it armed for itself. Enabling
interrupts and halting has to be one instruction pair, or the interrupt that
was going to end the wait lands in the gap and the CPU halts with nothing
left to wake it; `interrupt::enable_and_halt` is that pair, and the
architecture's interrupt shadow is why it works.

**Rationale.** Slice 4's idle thread removes the degenerate case by making
something always runnable, and inventing that thread here to avoid a six-line
branch would have pre-empted a decision this phase's charter gives to slice 4.

**Consequences.**

- A sleep on a machine whose clock never started returns at once. There is no
  deadline to wait for and no timer to be woken by, so a wait there would be a
  hang — and a kernel that hangs tells the harness nothing, which is worse
  than one that tells it something wrong.
- The polled keyboard path D-37 left behind was driven by whatever the tick
  woke. It now sleeps ten milliseconds at a time between drains, which is the
  same rate restated as the interval it always was, asked for by the only code
  that needs it.
- Two checks, one per branch, each broken once. A sleeper with nothing else to
  run comes back on time and the machine takes **one** timer interrupt while
  it waits, bounded on both sides so a conversion wrong in either direction
  shows. A sleeper with a competitor hands the CPU over, counted by a wake
  counter that only the sleeping-to-ready path moves.
- Still owed: nothing can be woken *early*, by an event rather than a
  deadline. That is `WaitQueue`, and it is slice 4's.
