# D-58 Sleeping: a deadline, a preserved lag, and a halt for the degenerate case

Status: Accepted, 2026-08-23.

**Decision.** A thread state, a deadline, and a saved lag. `sleep_ms`
charges the thread, records its leaving lag, marks it sleeping until an
absolute deadline, and hands the CPU to the policy's pick. Every scheduling
path wakes whatever is due before deciding anything. A woken thread returns
at the queue's fair-share point, offset by its saved lag; there D-53's clamp
acts against a sleeper.

The wait is a loop, not a single switch: something other than its own
deadline can wake a thread, and the deadline is what was promised.

**The degenerate case is chosen.** A thread that sleeps with nothing else
runnable keeps the CPU and halts on it: still running, still charged,
waiting on the timer it armed. Enabling interrupts and halting must be one
instruction pair. Otherwise the interrupt meant to end the wait lands in the
gap, and the CPU halts with nothing left to wake it.
`interrupt::enable_and_halt` is that pair, and the architecture's interrupt
shadow is why it works.

**Rationale.** Slice 4's idle thread removes the degenerate case by making
something always runnable; inventing it here to avoid a six-line branch
would pre-empt that slice's decision.

**Consequences.**

- A sleep on a machine whose clock never started returns at once. With no
  deadline and no timer, a wait there would hang, and a hung kernel tells
  the harness nothing, which is worse than telling it something wrong.
- The polled keyboard fallback sleeps ten milliseconds between drains.
