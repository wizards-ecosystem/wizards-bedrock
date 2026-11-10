# D-58 Sleeping: a deadline, a preserved lag, and a halt for the degenerate case

Status: Accepted, 2026-08-23.

**Context.** Tickless is worth building only if something wants to wait.
D-53 built lag-clamped placement (a sleeper keeps its debt and banks at most
one slice of credit) and noted that nothing could sleep yet, so the clamp
had no caller. This is the caller.

**Decision.** A thread state, a deadline, and a saved lag. `sleep_ms`
charges the thread, records its leaving lag, marks it sleeping until an
absolute deadline, and hands the CPU to the policy's pick. Every scheduling
path wakes whatever is due before deciding anything. A woken thread returns
at the queue's fair-share point, offset by its saved lag; there D-53's clamp
acts, at last against a sleeper.

The wait is a loop, not a single switch: something other than its own
deadline can wake a thread, and the deadline is what was promised.

**The degenerate case is chosen.** A thread that sleeps with nothing else
runnable keeps the CPU and halts on it: still running, still charged, waiting
on the timer it armed. Enabling interrupts and halting must be one instruction
pair. Otherwise the interrupt meant to end the wait lands in the gap, and the
CPU halts with nothing left to wake it. `interrupt::enable_and_halt` is that
pair, and the architecture's interrupt shadow is why it works.

**Rationale.** Slice 4's idle thread removes the degenerate case by making
something always runnable. Inventing that thread here to avoid a six-line
branch would have pre-empted a decision this phase's charter gives to slice
4.

**Consequences.**

- A sleep on a machine whose clock never started returns at once. With no
  deadline and no timer, a wait there would hang. A hung kernel tells the
  harness nothing, which is worse than telling it something wrong.
- The polled keyboard path D-37 left behind was driven by whatever the tick
  woke. It now sleeps ten milliseconds between drains: the same rate as
  the interval it always was, asked for by the only code that needs it.
- Two checks, one per branch, each broken once. A sleeper with nothing else
  to run comes back on time, and the machine takes **one** timer interrupt
  while it waits, bounded on both sides so a conversion wrong either way
  shows. A sleeper with a competitor hands the CPU over, counted by a wake
  counter only the sleeping-to-ready path moves.
- Still owed: nothing can be woken *early*, by an event instead of a
  deadline. That is `WaitQueue`, and it is slice 4's.
