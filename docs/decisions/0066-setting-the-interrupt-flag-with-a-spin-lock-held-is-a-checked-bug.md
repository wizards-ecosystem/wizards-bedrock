# D-66 Setting the interrupt flag with a spin lock held is a checked bug

Status: Accepted, 2026-08-23.

**Context.** The self-test hung on roughly one boot in ten, always after the
harness injected a keystroke, and always with the whole machine wedged. It
survived a 400-second budget, so it was not slowness; it happened in release
and debug alike; and disabling the blocked-reader checks made it stop.

Reading did not find it. What did was asking QEMU's monitor where the CPU was:
`RIP` fixed across three samples a second apart, `RFLAGS` with the interrupt
flag clear, and a disassembly showing a `pause`/`cmpxchg` pair against a byte
in `input::EVENTS`. The keyboard handler was spinning on the event queue's
lock, with interrupts off, for ever.

The holder was `read_key`:

```rust
let saved = interrupt::disable();
if let Some(keystroke) = EVENTS.lock().pop() {
    saved.restore();          // the guard is still alive here
    return keystroke;
}
```

The guard is a temporary in the `if let` scrutinee, so it lives to the closing
brace — past the `restore` that sets the interrupt flag. For the few
instructions in between, the queue is locked and interrupts are on. A keyboard
interrupt landing in that window runs `poll`, waits on the same lock, and
waits on a thread that can never resume to release it.

**Decision.** Three things, in order of what they buy.

The window closes: the pop becomes a statement of its own, so the guard is
dropped before the flag goes back.

Setting the flag with a spin lock held becomes a checked bug. `set_flag` is
the one place the kernel sets `IF`, and it now asserts the held count is zero;
`enable_and_halt`, which sets the flag in its own assembly, asserts the same.

Switching threads with one held becomes a checked bug too. `prepare` asserts
before it takes its own lock: a switch would leave the lock held by a context
nothing can schedule, which is the same stranding by another route.

**Rationale.** The rule was already written down — it is the argument
`SpinLock`'s doc comment makes, and the reason `ThreadLock` refuses a caller
holding one. What it lacked was an assert, so a single misplaced statement
could break it and pass the gate a hundred times.

`spinlocks_held` already existed for `ThreadLock`; the count was there and
nothing else asked it. The cost is a relaxed load and a branch on a path that
already does a locked increment.

**Consequences.**

- The reintroduced bug now panics on the first read of every boot, naming
  `interrupt.rs`. What took a hundred boots to see once is deterministic.
- A self-test kernel that panics halts rather than reporting, so `run_selftest`
  watches for the panic marker and fails in seven seconds instead of waiting
  out the timeout.
- On a timeout the harness now asks the monitor for the instruction pointer
  and disassembles around it. Both silent-hang diagnostics this bug needed are
  kept, because the next hang will need them too.
- Three asserts, three breaks: a lock held across a switch, a lock held across
  a halt, and the original bug restored.
- The panic path is exempt from the flag assert and not from the switch one: a
  panic reclaims locks it may find held, and never switches.
