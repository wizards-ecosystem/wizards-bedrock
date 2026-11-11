# D-59 A thread's first interrupt flag comes from its creator

Status: Accepted, 2026-08-23.

**Decision.** A new thread's interrupt flag is captured in `fabricate`, from
the thread's creator, and restored by the entry trampoline. It cannot be
captured at the switch, where the only flag left to read is the scheduler's
own.

**Rationale.** D-51 gave a new thread the switcher's flag, and a scheduler
clears the flag around every decision, so every thread first run by
`switch_to` started with interrupts disabled for good. A new thread holds no
lock and protects nothing, so no state it inherits justifies running
uninterruptible. The one thing worth inheriting is whether the machine
accepted interrupts when it was made. Its creator's flag says that, and is
correctly `false` on a machine where interrupts never came on, where
enabling them unconditionally would panic.

**Consequences.**

- `HANDOFF_INTERRUPTS` is gone, and with it the only entry on D-56's per-CPU
  list that was not per-CPU: it belongs to a slot, not a processor.
- A thread spawned from inside a critical section still starts
  uninterruptible. That is the inheritance rule, not an oversight; it is
  deterministic and documented, and nothing in the kernel does so.
