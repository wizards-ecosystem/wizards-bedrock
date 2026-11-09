# D-102 A fault the program caused kills the program

Status: Accepted (2026-08-24), amends D-31

**Context.** D-31 decided that a fault is terminal and reported, and that was
right when it was written: every fault was a kernel bug, and a kernel that
carries on after one is a kernel lying about its own state. Ring 3 changed the
premise (D-96). A `ud2`, a division by zero, or a touch of a page that is not
there can now be a *program* bug, and D-97 already reasons from exactly the
right principle for the copy path — *the program is the untrusted party*. The
trap path did not. D-101's audit named it the largest thing the slice owed.

**Decision.** A fault whose saved `CS` says ring 3 retires the calling thread
instead of stopping the machine, unless it is a fault that means the machine
itself is in trouble: a double fault means the fault path failed, a machine
check is the hardware reporting its own failure, and an NMI is not a program's
to cause. Those stay terminal whoever was running.

The privilege comes from the selector the CPU pushed, which is the hardware's
account of where the fault happened and cannot be forged by kernel code that
merely resembles a program.

**The program can now fault on purpose.** It reads one word of argument from
its own stack — written across the boundary with `copy_to_user`, which is how a
program will get everything it is ever given — and either calls the kernel or
executes `ud2`. Two behaviours from one image, and the check drives the second.

That the fault is *deliberate* is what makes it a check rather than a hope: an
accidental fault would be indeterminate about which fault, and `#UD` is the one
nothing else in this kernel raises.

**Rationale.** The alternative is what was there: a program can stop the
machine. That is not a security boundary, it is a shared fate, and every phase
after this one puts more untrusted code behind it.

The narrower question — whether to *report* the fault before retiring — is
answered by D-31's own rule: nothing logs from trap context, because the
console lock may be held by the code the trap interrupted. The frame is
recorded into atomics and read by whoever wants it, which is what
`last_breakpoint` and `last_ping` already do.

**Alternatives.** Keep faults terminal and let programs be trusted (rejected:
that is not a boundary); retire on *every* fault including double faults
(rejected: a double fault means the fault path itself failed, and continuing is
a claim the kernel cannot support); deliver the fault to the program as a signal
(rejected for now — D-80 reserves the completion plane for them, but there is
no signal record or queue yet and the receiving thread does not survive this
phase).

**Consequences.**

- One hundred and three named machine checks. Breaking the retire arm restores
  the old behaviour exactly, and the report it prints is the evidence:
  *invalid opcode (vector 6) at 0x400000020006, cs 0x33* — the program's own
  instruction, at ring 3.
- **One half of the condition has no check, and cannot have one.** That a
  *kernel* fault stays terminal is proved by the kernel panicking, which is not
  a green outcome — so removing the ring-3 test leaves every check passing. The
  hazard it guards is a kernel bug swallowed rather than reported, which is
  worse than the panic this replaces, and it is guarded by review rather than by
  the gate.
- A retired thread keeps its slot and its mapped pages for the life of the boot.
  Nothing reclaims one yet, and the faulting program makes that more visible
  rather than more urgent: `spawn`'s counterpart is what will.
- D-31 stands for kernel faults, which is what it was written about.
