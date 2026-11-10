# D-66 Setting the interrupt flag with a spin lock held is a checked bug

Status: Accepted, 2026-08-23.

**Context.** The self-test hung on roughly one boot in ten, always after the
harness injected a keystroke, with the whole machine wedged. Surviving a
400-second budget ruled out slowness. It hit release and debug alike, and
disabling the blocked-reader checks stopped it.

Reading did not find it; asking QEMU's monitor where the CPU was did. `RIP`
held across three samples a second apart, `RFLAGS` had the interrupt flag
clear, and the disassembly showed a `pause`/`cmpxchg` pair against a byte in
`input::EVENTS`. The keyboard handler was spinning on the event queue's lock,
with interrupts off, for ever.

The holder was `read_key`:

```rust
let saved = interrupt::disable();
if let Some(keystroke) = EVENTS.lock().pop() {
    saved.restore();          // the guard is still alive here
    return keystroke;
}
```

The guard is a temporary in the `if let` scrutinee, so it lives to the closing
brace, past the `restore` that sets the interrupt flag. For the few
instructions between, the queue is locked with interrupts on. A keyboard
interrupt there runs `poll` and waits on the same lock, held by a thread that
can never resume to release it.

**Decision.** Three things, in order of what they buy.

The window closes: the pop becomes its own statement, so the guard drops
before the flag returns.

Setting the flag with a spin lock held becomes a checked bug. `set_flag`, the
one place the kernel sets `IF`, now asserts the held count is zero.
`enable_and_halt`, which sets the flag in its own assembly, asserts the same.

Switching threads with one held becomes a checked bug too: `prepare` asserts
before taking its own lock. A switch would leave the lock held by a context
nothing can schedule, the same stranding by another route.

**Rationale.** The rule was already written: `SpinLock`'s doc comment argues
it, and `ThreadLock` refuses a caller holding one for that reason. Without an
assert, one misplaced statement could break it and pass the gate a hundred
times.

`spinlocks_held` already existed for `ThreadLock`, unused elsewhere. The cost
is a relaxed load and a branch on a path that already does a locked increment.

**Consequences.**

- The reintroduced bug now panics on the first read of every boot, naming
  `interrupt.rs`. What took a hundred boots to see once is deterministic.
- A panicking self-test kernel halts without reporting, so `run_selftest`
  watches for the panic line and fails in seven seconds, not at the timeout.
- On a timeout the harness asks the monitor for the instruction pointer and
  disassembles around it; both silent-hang diagnostics this bug needed stay
  for the next hang.
- Three asserts, three breaks: a lock held across a switch, a lock held across
  a halt, and the original bug restored.
- The panic path is exempt from the flag assert but not the switch one: a
  panic reclaims locks it may find held, and never switches.
