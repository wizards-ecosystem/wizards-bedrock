# D-51 The context switch is a naked function, and a switch is a safe call

Status: Accepted, 2026-08-22. Amended by D-59 and D-76.

**Context.** `008-research-notes-2.md` §1 verified `#[unsafe(naked)]` and
`naked_asm!` against the pinned toolchain and named Phase 4's context switch
as the place to adopt them. The only other assembly in the kernel is the
interrupt trampoline, a `global_asm!` blob outside every mechanism `005`
mandates: no declared type, no `///`, no `# Safety`.

**Options.** Another `global_asm!` blob (a symbol the type system cannot
see); a naked function (a Rust item with a declared type and a contract);
convert the interrupt trampoline to match (churn on the repo's most
battle-tested assembly, for no behavioural change).

**Decision.** `wost::task::switch` is a naked function: push the six
callee-saved registers, store `rsp`, load the other, pop, `ret`. Nothing else
crosses: the ABI lets a call clobber the rest, and without SSE there is no
floating-point state. The interrupt trampoline is **not** converted: that is a
judgement about churn, and it may be revisited when the trampoline next changes
for a reason of its own.

The switch is `unsafe` and private. What the kernel calls (`switch_to`,
`exit_to`, `fabricate`) is safe, on the grounds that make `activate` safe
(D-45): every value the switch loads can only have come from this module. A
saved pointer is written by a previous switch or by `fabricate`, and
`fabricate` writes only on a stack the arena mapped and `init` walked in the
live tables. An exited slot is marked dead and refused by every later
switch, so nothing resumes onto a stack since handed to a new thread. The
kernel never holds a context value: it names slots by index, and the table
decides what each index may do.

**Rationale.** The sound-wrapper principle `005` holds every `wost` export to
asks whether any caller can reach undefined behaviour, not whether the
assembly is right. A public `Context` value would make that unanswerable:
two of them for one stack is a switch into a frame something else stands
on. Private state with only checked transitions answers it. The naked
function suits the assembly for §1's reason: it is an item, so the lints
reach it.

**Consequences.**

- The interrupt flag is clear across every switch and restored by a context
  that resumes. The founding implementation gave a new thread the
  switcher's flag; D-59 supersedes that rule and captures the creator's flag
  in `fabricate` instead.
- The founding fabricated frame was six zero words plus `thread_start`'s
  address. D-76 found that seven-word frame misaligned every new thread and
  added an eighth padding word; the current ABI argument is in D-76.
- The first break-once attempt, deleting the `r12` push and pop, tested the
  wrong thing: it changed the frame size, so the fabricated frame's `ret`
  popped a zero and the machine faulted. Saving `rax` in `r12`'s place is the
  break that tests preservation, and it turned the register check red. A
  break that crashes proves less than one that fails.
- Kernel-side, `THREADS` is a leaf lock and every switch happens after its
  guard drops. `wost`'s own table lock is taken inside `switch_to`, never
  under the kernel's. The lock order is in `009-multitasking.md`.
- Not yet enforced: that a thread does not yield while holding a lock. It
  cannot corrupt memory (the next thread spins with the flag it inherited),
  but it can hang, and D-55 (the context token) is where the type system
  takes it over.
