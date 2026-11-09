# D-99 The ring-0 stack moves with the thread

Status: Accepted (2026-08-24), lifts D-96's limitation

**Context.** D-96 set `RSP0` once, to a stack of its own, and wrote the
consequence down rather than hiding it: **one thread may be in ring 3 at a
time**. A second would have taken its trap on a stack the first was standing
on, and the corruption would have been silent until the two frames overlapped.

**Decision.** `RSP0` is written by `prepare`, the one function every context
switch passes through, from the stack top the task table already holds for the
slot being switched to. A thread carries its landing with it.

The boot slot has no arena stack and its zero is skipped — it is also the one
thread that cannot be in ring 3, being the one that put the others there.

**The evidence is not a type, and that is deliberate.** `set_kernel_stack` takes
a `MappedStack` because D-95 refused a bare address from outside the arch
module. The switch has no `MappedStack` to offer: these are arena stacks, mapped
before activation by `map_stacks`. What stands behind them instead is
`task::init`, which walks every stack and every guard in the arena and refuses
to fabricate a thread if one disagrees (D-52). So `write_kernel_stack` is
`pub(super)`, reachable only from inside the module where that walk happened,
and the bare-address entry stays refused everywhere else.

**Rationale.** Putting it in `prepare` rather than in the kernel's scheduler is
the whole point. There are three ways into a switch — `switch_to`,
`switch_from_trap`, `exit_to` — and a scheduler that set `RSP0` at two of them
would leave the third as a silent privilege bug. `prepare` is where the slot is
validated and `CURRENT` is stored; adding one more thing that must be true of
the incoming thread belongs beside them.

**Consequences.**

- One hundred named machine checks. The new one spawns a second ring-3 thread
  and requires that its trap landed somewhere other than the first's, which a
  single shared stack cannot satisfy. Breaking the write turns *three* checks
  red — the two crossings and the buffer clear — because `enter_user` refuses
  when `RSP0` is zero rather than entering ring 3 and faulting (D-96).
- The dedicated user-trap stack window is gone, and with it the fourth layout
  derived from the arena's end.
- **This is not isolation.** The two programs share one address space and one
  set of tables; what is separate is the *kernel* side of two simultaneous
  crossings. A second address space is the rest of this slice, and until it
  exists "another program" means "another pair of addresses".
- Eager XSAVE is now reachable and still absent: two threads can be in ring 3,
  so two can have floating-point state to lose. Nothing uses it yet — the
  program is four bytes — but the gap is real from here rather than
  hypothetical, and it arrives with the processes in the rest of this slice.
