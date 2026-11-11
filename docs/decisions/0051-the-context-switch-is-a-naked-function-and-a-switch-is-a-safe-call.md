# D-51 The context switch is a naked function, and a switch is a safe call

Status: Accepted, 2026-08-22. Amended by D-59 and D-76.

**Decision.** `wost::task::switch` is a naked function: push the six
callee-saved registers, store `rsp`, load the other, pop, `ret`. Nothing
else crosses: the ABI lets a call clobber the rest, and without SSE there is
no floating-point state. The interrupt trampoline is **not** converted: that
is a judgement about churn, and it may be revisited when the trampoline next
changes for a reason of its own.

The switch is `unsafe` and private. What the kernel calls (`switch_to`,
`exit_to`, `fabricate`) is safe, on the grounds that make `activate` safe
(D-45): every value the switch loads can only have come from this module. A
saved pointer is written by a previous switch or by `fabricate`, and
`fabricate` writes only on a stack the arena mapped and `init` walked in the
live tables. An exited slot is marked dead and refused by every later
switch, so nothing resumes onto a stack since handed to a new thread. The
kernel never holds a context value: it names slots by index, and the table
decides what each index may do.

**Rationale.** A public `Context` value would let a safe caller hold two for
one stack, a switch into a frame something else stands on. A naked function,
unlike a `global_asm!` blob, is an item with a declared type and a contract,
so the lints reach it (`008-research-notes-2.md` §1).

**Consequences.**

- The interrupt flag is clear across every switch and restored by a context
  that resumes. D-59 supersedes the founding rule that a new thread got the
  switcher's flag, and captures the creator's flag in `fabricate` instead.
- D-76 found the founding seven-word fabricated frame misaligned every new
  thread and added an eighth padding word; the current ABI argument is in
  D-76.
- A break that crashes proves less than one that fails: register
  preservation is tested by saving `rax` in `r12`'s place, not by deleting a
  push.
- Kernel-side, `THREADS` is a leaf lock and every switch happens after its
  guard drops. `wost`'s own table lock is taken inside `switch_to`, never
  under the kernel's. The lock order is in `009-multitasking.md`.
- Not yet enforced: that a thread does not yield while holding a lock. It
  cannot corrupt memory (the next thread spins with the flag it inherited),
  but it can hang, and D-55 (the context token) is where the type system
  takes it over.
