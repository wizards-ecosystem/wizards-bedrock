# D-99 The ring-0 stack moves with the thread

Status: Accepted, 2026-08-24. Lifts D-96's limitation.

**Decision.** `RSP0` is written by `prepare`, the one function every context
switch passes through, from the stack top the task table holds for the incoming
slot. A thread carries its landing with it, so more than one thread may be in
ring 3 (D-96 allowed one).

The boot slot has no arena stack and its zero is skipped. It is also the one
thread that cannot be in ring 3, since it put the others there.

**The evidence is not a type.** `set_kernel_stack` takes a `MappedStack` because
D-95 refused a bare address from outside the arch module. The switch has no
`MappedStack` to offer: these are arena stacks, and behind them stands
`task::init`, which walks the arena's stacks and guards and refuses to
fabricate a thread if one disagrees (D-52). So `write_kernel_stack` is
`pub(super)`, reachable only from the module where that walk happened. The
bare-address entry stays refused everywhere else.

**Rationale.** The write belongs in `prepare`, not in the kernel's scheduler.
There are three ways into a switch: `switch_to`, `switch_from_trap` and
`exit_to`. A scheduler that set `RSP0` at two of them would leave the third as a
silent privilege bug.

**Consequences.**

- The check spawns a second ring-3 thread and requires its trap to land apart
  from the first's, which one shared stack cannot satisfy. `enter_user` refuses
  when `RSP0` is zero instead of entering ring 3 and faulting (D-96).
- **This is not isolation.** The two programs share one address space and one
  set of tables; only the *kernel* side of two simultaneous crossings is
  separate.
- Eager XSAVE is now reachable and still absent: two threads can be in ring 3,
  so two can have floating-point state to lose. The gap is real from D-99
  onward rather than hypothetical; eager XSAVE arrives with the processes.
