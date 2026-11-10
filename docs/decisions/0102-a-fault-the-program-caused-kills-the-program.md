# D-102 A fault the program caused kills the program

Status: Accepted, 2026-08-24. Amends D-31.

**Context.** D-31 made a fault terminal and reported. That was right when
written: every fault was a kernel bug, and a kernel carrying on after one lies
about its own state. Ring 3 changed the premise (D-96). A `ud2`, a division by
zero or a touch of a missing page can now be a *program* bug. D-97 already
reasons from the right principle for the copy path: *the program is the
untrusted party*. The trap path did not, and D-101's audit named it the largest
thing the slice owed.

**Decision.** A fault whose saved `CS` says ring 3 retires the calling thread
instead of stopping the machine, unless the fault means the machine itself is in
trouble. A double fault means the fault path failed. A machine check is the
hardware reporting its own failure, and an NMI is not a program's to cause.
Those stay terminal whoever was running.

The privilege comes from the selector the CPU pushed: the hardware's account of
where the fault happened, which kernel code resembling a program cannot forge.

**The program can now fault on purpose.** It reads one word of argument from its
own stack, written across the boundary with `copy_to_user`, the way a program
will get everything it is given. It then either calls the kernel or executes
`ud2`. One image has two behaviours, and the check drives the second.

The fault being *deliberate* makes it a check: an accidental fault would leave
open which fault, and `#UD` is the one nothing else in this kernel raises.

**Rationale.** The alternative is what was there: a program can stop the
machine. That is shared fate, not a security boundary, and every later phase
puts more untrusted code behind it.

Whether to *report* the fault before retiring is answered by D-31's own rule:
nothing logs from trap context, since the interrupted code may hold the console
lock. The frame is recorded into atomics and read by whoever wants it, as
`last_breakpoint` and `last_ping` already do.

**Alternatives.** Keep faults terminal and trust programs (rejected: that is no
boundary). Retire on *every* fault including double faults (rejected: the fault
path itself failed, and the kernel cannot support continuing). Deliver the fault
to the program as a signal (rejected for now: D-80 reserves the completion plane
for them, but no signal record or queue exists yet). The receiving thread also
does not survive this phase.

**Consequences.**

- One hundred and three named machine checks. Breaking the retire arm restores
  the old behaviour exactly, and its report is the evidence: *invalid opcode
  (vector 6) at 0x400000020006, cs 0x33*, the program's own instruction, at ring 3.
- **One half of the condition has no check, and cannot have one.** A *kernel*
  fault staying terminal is proved by a panic, which is not a green outcome.
  Removing the ring-3 test therefore leaves every check passing. The hazard is a
  kernel bug swallowed instead of reported, worse than the panic this replaces.
  Review guards it, not the gate.
- A retired thread keeps its slot and its mapped pages for the life of the boot.
  Nothing reclaims one yet; the faulting program makes that more visible, not
  more urgent. `spawn`'s counterpart will.
- D-31 stands for kernel faults, which is what it was written about.
