# D-101 The third audit: ring 3 could switch SMAP off, and the flush was never executed

Status: Accepted, 2026-08-24.

**Context.** A third adversarial pass covered the privilege boundary itself:
`enter_user`'s `iretq` frame, the user copy, the loader parsing initrd bytes,
the TSS padding and the `VERW` tail. The two before it each found undefined
behaviour, one introduced by the other's repair. This one found eight defects;
the first is the worst of all three.

**Ring 3 could turn SMAP off for the whole kernel.** `EFLAGS.AC` is writable at
CPL 3, and interrupt delivery clears `TF`, `NT`, `RF` and `VM`, **not `AC`**. A
program that does `pushfq`, sets bit 18, `popfq`, `int 0x80` enters with SMAP
disabled. It stays disabled for the whole handler *and for every thread switched
to afterwards*, since a context switch restores no flags. The entry stub had no
`clac`; Linux emits `ASM_CLAC` atop every entry point for this.

Nothing here dereferences a user pointer outside `usercopy`, so nothing was
exploitable, but the slice's central mitigation was unarmed on the path that
matters. D-97's three-instruction window was false in both directions: it is the
copy *plus any interrupt taken inside it*, and ring 3 could open it with no copy
at all.

**The program in the image now does it.** Fourteen bytes instead of four set
`AC`, then call: a hostile initrd in the gate on every boot. The check asserts
both halves, since either alone passes for the wrong reason. The saved flags
must show the program set `AC`, and the handler must have run with it clear.
Removing the `clac` turns it red.

**The `VERW` flush was shipped unexecuted, the state D-98 was written to
escape.** D-98 put it in the entry stub's tail, reached on a *return* to ring 3.
The kernel-to-ring-3 transition is `leave_for_user`'s own `iretq`, which had
none, and the syscall handler retires its caller, so no return to ring 3
happens. D-98's consequence prose claimed the crossing "passes through it"; it
does not. The flush now sits on both transitions. The kernel-to-user one matters
more, after the kernel parsed an initrd, walked page tables and touched the
frame allocator.

**`enter_user` accepted another thread's kernel stack.** `prepare` skips the
`RSP0` write for the boot thread's slot, which has no arena stack. After a
switch to it, `RSP0` named whichever arena thread ran last.
`kernel_stack() == 0` passed, and the first trap out of ring 3 would have
written a frame onto another thread's stack. D-99 defended this with prose: *"it
is also the one thread that cannot be in ring 3"*. Nothing enforced it, yet
`enter_user` is a public safe function that D-76 requires to hold against every
safe caller. It now compares against *this* thread's stack top and asserts no
lock is held, as `prepare` and `continue_on_stack` do. The function that never
returns needed that rule most.

**The loader did unchecked arithmetic on two attacker-controlled words:**
`segment.address() + shift` and `elf.entry() + shift`. Debug builds panic on
initrd bytes, against D-100's claim that the loader "never panics because the
bytes are not the kernel's", true of `libs/elf64` and false of its caller.
Release builds wrap past the `NotUserMemory` refusal. `contents_fit` does this
correctly one function away.

**A check could not fail, again.** *a thread crosses into ring three* ended in a
`retired` conjunct awaiting a slot a retired thread never returns. Its fallback
comparison was true of the checking thread whatever happened. Deleting
`retire_from_trap`'s switch left it green. It counts retirements now, and
breaking the count turns it red. The third audit in a row found this failure
mode, in a commit claiming every new check was broken.

**Four documents said false things**, three of them naming the mechanism wrong.
D-96 and `011` called it *a trap gate* four times. The code builds an
**interrupt gate at DPL 3**, and `libs/x86-tables` states that a trap gate "is
deliberately unavailable". `011` still carried D-96's one-thread-in-ring-3
limitation that D-99 had lifted. `xtask`'s program builder claimed a file-offset
congruence it does not hold, describing the bug it says it avoided. And
`USER_END` was open-coded twice in the checks, in the same commit whose ADR says
*"named once. Nothing computes it."*

**Rationale.** The first finding generalises. Every mitigation in this phase was
reasoned from the kernel's side: is the window short, is the check before the
copy, is the range right. The one that mattered was a bit the *program*
controls, surviving a transition nobody had asked what it preserved. The audit
found it in the manual's list of what interrupt delivery clears, by noticing
what was absent.

**Consequences.**

- One hundred and two named machine checks. The new one is driven by a hostile
  program that ships in the image.
- `clac` costs a compare and a branch on every trap entry, because this kernel
  has no instruction patching. `clac` is a `#UD` where SMAP is absent, which the
  degraded boot (D-90) exercises.
- **Two findings are recorded and not fixed.** `paging::translate` returns the
  *leaf* entry's flags without intersecting the parents', so four callers state
  a stronger property than they check. It holds today only because
  `Entry::table` always sets `PRESENT | WRITABLE` and `set` upgrades parents to
  `USER`. Once a parent is read-only or NX, `admits` will pass and the copy will
  take a terminal fault. And **a ring-3 fault still stops the machine**: a `ud2`
  or an unmapped touch in ring 3 panics the kernel. D-31 was written when a
  fault was necessarily a kernel bug. This is now the largest thing the slice
  owes, recorded in `011`.
