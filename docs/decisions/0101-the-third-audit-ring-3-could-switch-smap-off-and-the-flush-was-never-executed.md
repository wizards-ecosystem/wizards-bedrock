# D-101 The third audit: ring 3 could switch SMAP off, and the flush was never executed

Status: Accepted, 2026-08-24.

**Decision.** A third adversarial pass covered the privilege boundary itself.
Its fixes:

- **Every trap entry executes `clac`.** `EFLAGS.AC` is writable at CPL 3, and
  interrupt delivery clears `TF`, `NT`, `RF` and `VM`, **not `AC`**, so a
  program that set bit 18 and trapped entered with SMAP disabled for the
  handler *and for every thread switched to afterwards*. Nothing dereferenced
  a user pointer outside `usercopy`, so nothing was exploitable. D-97's
  three-instruction window was false in both directions: it is the copy *plus
  any interrupt taken inside it*, and ring 3 could open it with no copy at all.
- **The program in the image sets `AC`, then calls**: a hostile initrd in the
  gate on every boot. The check asserts both halves, since either alone passes
  for the wrong reason: the saved flags show the program set `AC`, and the
  handler ran with it clear.
- **The `VERW` flush sits on both transitions.** D-98 put it on a *return* to
  ring 3, but the kernel-to-ring-3 transition is `leave_for_user`'s own `iretq`
  and the syscall handler retires its caller, so the flush was shipped
  unexecuted. The kernel-to-user one matters more.
- **`enter_user` compares `RSP0` against *this* thread's stack top** and
  asserts no lock is held. D-99 defended the boot thread with prose (*"it is
  also the one thread that cannot be in ring 3"*), yet `enter_user` is a public
  safe function that D-76 requires to hold against every safe caller.
- **The loader's arithmetic on attacker-controlled words is checked**:
  `segment.address() + shift` and `elf.entry() + shift` panicked in debug and
  wrapped past `NotUserMemory` in release, against D-100's claim that the
  loader never panics.
- *a thread crosses into ring three* counts retirements; its old fallback was
  true whatever happened.
- **The gate is an interrupt gate at DPL 3**, not the *trap gate* D-96 and `011`
  named; `libs/x86-tables` states that a trap gate "is deliberately
  unavailable". `USER_END` is named once.

**Rationale.** Every mitigation in this phase was reasoned from the kernel's
side. The one that mattered was a bit the *program* controls, surviving a
transition nobody had asked what it preserved, found by noticing what was
absent from the manual's list of what interrupt delivery clears.

**Consequences.**

- `clac` costs a compare and a branch on every trap entry, because this kernel
  has no instruction patching. `clac` is a `#UD` where SMAP is absent, which the
  degraded boot (D-90) exercises.
- **Two findings are recorded and not fixed.** `paging::translate` returns the
  *leaf* entry's flags without intersecting the parents', so four callers state
  a stronger property than they check. It holds today only because
  `Entry::table` always sets `PRESENT | WRITABLE` and `set` upgrades parents to
  `USER`. And **a ring-3 fault still stops the machine**: D-31 was written when
  a fault was necessarily a kernel bug. This is now the largest thing the slice
  owes, recorded in `011`.
