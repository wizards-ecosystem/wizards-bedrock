# D-66 Setting the interrupt flag with a spin lock held is a checked bug

Status: Accepted, 2026-08-23.

**Decision.** Three things, in order of what they buy.

The window closes: in `read_key` the pop becomes its own statement, so the
guard drops before the flag returns. (As an `if let` scrutinee temporary it
lived past the `restore`, leaving the queue locked with interrupts on, and a
keyboard interrupt there spun on the lock for ever.)

Setting the flag with a spin lock held becomes a checked bug. `set_flag`,
the one place the kernel sets `IF`, asserts the held count is zero.
`enable_and_halt`, which sets the flag in its own assembly, asserts the
same.

Switching threads with one held becomes a checked bug too: `prepare` asserts
before taking its own lock. A switch would leave the lock held by a context
nothing can schedule, the same stranding by another route.

**Rationale.** The rule was already written: `SpinLock`'s doc comment argues
it, and `ThreadLock` refuses a caller holding one for that reason. Without
an assert, one misplaced statement could break it and pass the gate a
hundred times. The cost is a relaxed load and a branch on a path that
already does a locked increment.

**Consequences.**

- A panicking self-test kernel halts without reporting, so `run_selftest`
  watches for the panic line and fails at once, not at the timeout.
- On a timeout the harness asks the monitor for the instruction pointer and
  disassembles around it; both silent-hang diagnostics stay for the next
  hang.
- The panic path is exempt from the flag assert but not the switch one: a
  panic reclaims locks it may find held, and never switches.
