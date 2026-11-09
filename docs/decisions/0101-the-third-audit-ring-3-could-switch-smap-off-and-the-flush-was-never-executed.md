# D-101 The third audit: ring 3 could switch SMAP off, and the flush was never executed

Status: Accepted (2026-08-24)

**Context.** A third adversarial pass, over the privilege boundary itself —
`enter_user`'s `iretq` frame, the user copy, the loader parsing initrd bytes,
the TSS padding, and the `VERW` tail. The two before it each found real
undefined behaviour, one of them introduced by the other's repair. This one
found eight defects, and the first is the worst thing any of the three has
turned up.

**Ring 3 could turn SMAP off for the whole kernel.** `EFLAGS.AC` is writable at
CPL 3, and interrupt delivery clears `TF`, `NT`, `RF` and `VM` — **not `AC`**.
So a program that does `pushfq`, sets bit 18, `popfq`, `int 0x80` lands in the
entry path with SMAP already disabled, and it stays disabled for the whole
handler *and for every thread switched to afterwards*, because a context switch
restores no flags. The entry stub had no `clac`. Linux emits `ASM_CLAC` at the
top of every entry point for precisely this.

Nothing in this kernel dereferences a user pointer outside `usercopy`, so
nothing was exploitable — but the mitigation the entire slice is built around
was not armed on the path that matters, and D-97's central claim about a
three-instruction window was false in both directions: the window is bounded by
the copy *plus any interrupt taken inside it*, and it could be opened from ring
3 with no copy at all.

**The program in the image now does it.** Fourteen bytes instead of four: set
`AC`, then call. A hostile initrd, in the gate, on every boot. The check
asserts both halves, because either alone passes for the wrong reason — the
saved flags must show the program really did set `AC`, and the handler must
have been running with it clear. Removing the `clac` turns it red.

**The `VERW` flush was shipped unexecuted, which is the state D-98 was written
to escape.** D-98 put it in the entry stub's tail, reached on a *return* to
ring 3. But the kernel-to-ring-3 transition is `leave_for_user`'s own `iretq`,
which had none — and the syscall handler retires its caller, so there is no
return to ring 3 at all. D-98's consequence prose claimed the crossing "passes
through it"; it does not. The flush now sits on both transitions, and the
kernel-to-user one is the more important: the kernel has just parsed an initrd,
walked page tables and touched the frame allocator.

**`enter_user` accepted another thread's kernel stack.** `prepare` skips the
`RSP0` write for a slot with no arena stack — the boot thread's — so after a
switch to it, `RSP0` still named whichever arena thread ran last.
`kernel_stack() == 0` passed, and the first trap out of ring 3 would have
written a frame onto a stack another thread was standing on. D-99 defended this
with prose: *"it is also the one thread that cannot be in ring 3"*. Nothing
enforced it, and `enter_user` is a public safe function, which D-76 requires to
hold against every safe caller. It now compares against *this* thread's stack
top, and asserts no lock is held — the rule `prepare` and `continue_on_stack`
both enforce, missing from the one function with the strongest reason for it,
since it never returns.

**The loader did unchecked arithmetic on two attacker-controlled words.**
`segment.address() + shift` and `elf.entry() + shift`. In debug that panics the
kernel on initrd bytes — against D-100's claim that the loader "never panics
because the bytes are not the kernel's", which was true of `libs/elf64` and
false of its caller. In release it wraps past the `NotUserMemory` refusal
entirely. `contents_fit` does this correctly one function away.

**A check could not fail, again.** *a thread crosses into ring three* ended with
a `retired` conjunct that waited for a slot to come back — which a retired
thread never returns — and fell back to a comparison true of the checking
thread whatever happened. Deleting `retire_from_trap`'s switch left it green.
It counts retirements now, and breaking the count turns it red. That is the
third audit in a row to find this failure mode, in a commit claiming every new
check was broken.

**Four documents said false things**, including three that named the mechanism
wrong. D-96 and `011` called it *a trap gate* four times; the code builds an
**interrupt gate at DPL 3**, and `libs/x86-tables` says in as many words that a
trap gate "is deliberately unavailable". `011` still carried D-96's
one-thread-in-ring-3 limitation that D-99 had lifted. `xtask`'s program builder
claimed a file-offset congruence it does not hold — describing the bug it says
it avoided. And `USER_END` was open-coded twice in the checks, in the same
commit whose ADR says *"named once. Nothing computes it."*

**Rationale.** The finding worth generalising is the first one. Every mitigation
in this phase was reasoned about from the kernel's side — is the window short,
is the check before the copy, is the range right — and the one that mattered was
a bit the *program* controls, surviving a transition nobody had asked what it
preserved. The audit found it by reading the manual's list of what interrupt
delivery clears and noticing what was absent.

**Consequences.**

- One hundred and two named machine checks. The new one is driven by a hostile
  program that ships in the image.
- `clac` costs a compare and a branch on every trap entry, because this kernel
  has no instruction patching and `clac` is a `#UD` where SMAP is absent —
  which the degraded boot (D-90) exercises.
- **Two findings are recorded and not fixed.** `paging::translate` returns the
  *leaf* entry's flags and never intersects the parents', so four callers state
  a stronger property than they check; it holds today only because
  `Entry::table` always sets `PRESENT | WRITABLE` and `set` upgrades parents to
  `USER`, and the day a parent is read-only or NX, `admits` will pass and the
  copy will take a terminal fault. And **a ring-3 fault still stops the
  machine** — a `ud2` or an unmapped touch in ring 3 panics the kernel, because
  D-31 was written when a fault was necessarily a kernel bug. That is now the
  largest thing this slice owes, and it is in `011`'s record.
