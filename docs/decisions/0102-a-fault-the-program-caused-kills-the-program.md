# D-102 A fault the program caused kills the program

Status: Accepted, 2026-08-24. Amends D-31.

**Decision.** A fault whose saved `CS` says ring 3 retires the calling thread
instead of stopping the machine, unless the fault means the machine itself is in
trouble. A double fault means the fault path failed. A machine check is the
hardware reporting its own failure, and an NMI is not a program's to cause.
Those stay terminal whoever was running.

The privilege comes from the selector the CPU pushed: the hardware's account of
where the fault happened, which kernel code resembling a program cannot forge.

**The program can fault on purpose.** It reads one word of argument from its
own stack, written across the boundary with `copy_to_user`, then either calls
the kernel or executes `ud2`. The fault being *deliberate* makes it a check:
`#UD` is the one fault nothing else in this kernel raises.

**Rationale.** D-31 was right when every fault was a kernel bug; ring 3 changed
the premise (D-96), and *the program is the untrusted party* (D-97). A program
that can stop the machine is shared fate, not a security boundary. The fault
is not logged before retiring, by D-31's own rule: nothing logs from trap
context, since the interrupted code may hold the console lock. The frame is
recorded into atomics and read by whoever wants it.

**Alternatives.** Keep faults terminal and trust programs (rejected: that is no
boundary). Retire on *every* fault including double faults (rejected: the fault
path itself failed, and the kernel cannot support continuing). Deliver the fault
to the program as a signal (rejected for now: D-80 reserves the completion plane
for them, but no signal record or queue exists yet). The receiving thread also
does not survive this phase.

**Consequences.**

- **One half of the condition has no check, and cannot have one.** A *kernel*
  fault staying terminal is proved by a panic, which is not a green outcome.
  Removing the ring-3 test therefore leaves every check passing. The hazard is a
  kernel bug swallowed instead of reported, worse than the panic this replaces.
  Review guards it, not the gate.
- A retired thread keeps its slot and its mapped pages for the life of the boot.
  Nothing reclaims one yet.
- D-31 stands for kernel faults, which is what it was written about.
