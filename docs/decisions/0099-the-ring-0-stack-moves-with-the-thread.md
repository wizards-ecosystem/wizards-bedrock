# D-99 The ring-0 stack moves with the thread

Status: Accepted, 2026-08-24. Lifts D-96's limitation.

**Context.** D-96 set `RSP0` once, to a stack of its own, and recorded the
consequence: **one thread may be in ring 3 at a time**. A second would have
taken its trap on the first one's stack, silently, until the two frames
overlapped.

**Decision.** `RSP0` is written by `prepare`, the one function every context
switch passes through, from the stack top the task table holds for the incoming
slot. A thread carries its landing with it.

The boot slot has no arena stack and its zero is skipped. It is also the one
thread that cannot be in ring 3, since it put the others there.

**The evidence is not a type.** `set_kernel_stack` takes a `MappedStack` because
D-95 refused a bare address from outside the arch module. The switch has no
`MappedStack` to offer: these are arena stacks, mapped before activation by
`map_stacks`. Behind them stands `task::init`, which walks the arena's stacks
and guards and refuses to fabricate a thread if one disagrees (D-52). So
`write_kernel_stack` is `pub(super)`, reachable only from the module where that
walk happened. The bare-address entry stays refused everywhere else.

**Rationale.** The write belongs in `prepare`, not in the kernel's scheduler.
There are three ways into a switch: `switch_to`, `switch_from_trap` and
`exit_to`. A scheduler that set `RSP0` at two of them would leave the third as a
silent privilege bug. `prepare` validates the slot and stores `CURRENT`, and
this belongs beside them.

**Consequences.**

- One hundred named machine checks. The new one spawns a second ring-3 thread
  and requires its trap to land apart from the first's, which one shared stack
  cannot satisfy. Breaking the write turns *three* checks red: the two crossings
  and the buffer clear. `enter_user` refuses when `RSP0` is zero instead of
  entering ring 3 and faulting (D-96).
- The dedicated user-trap stack window is gone, and with it the fourth layout
  derived from the arena's end.
- **This is not isolation.** The two programs share one address space and one
  set of tables; only the *kernel* side of two simultaneous crossings is
  separate. A second address space is the rest of this slice. Until it exists,
  "another program" means "another pair of addresses".
- Eager XSAVE is now reachable and still absent. Two threads can be in ring 3,
  so two can have floating-point state to lose. Nothing uses it yet (the program
  is four bytes), but the gap is real from here rather than hypothetical. Eager
  XSAVE arrives with the processes in the rest of this slice.
