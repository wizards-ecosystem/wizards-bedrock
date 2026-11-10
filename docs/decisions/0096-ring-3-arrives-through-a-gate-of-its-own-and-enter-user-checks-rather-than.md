# D-96 Ring 3 arrives through a gate of its own, and `enter_user` checks rather than trusts

Status: Accepted, 2026-08-24. Its one-thread limit lifted by D-99; corrected and hardened by D-101.

**Context.** D-81 shaped the entry path and left open whether the synchronous
call set arrives by `syscall`/`sysret` or a trap gate. `sysret`'s non-canonical
`RCX` hazard is a known CVE class costing a branch on every return; a trap gate
costs more per call. *The measurement that would settle it is meaningless under
TCG* (`010` §8).

**Decision. An interrupt gate at DPL 3, and the reason is not performance.**
(The first version of this entry called it "a trap gate". That was wrong twice:
an *interrupt* gate clears the flag on entry, and `libs/x86-tables` says a trap
gate "is deliberately unavailable". Corrected by D-101.) `syscall` needs
`swapgs`, `swapgs` needs a per-CPU block, and a per-CPU block reached through
`gs:` is a second unproven mechanism in the same commit as the privilege
boundary. A gate reached through the interrupt table needs none of it: the CPU
loads `RSP0` from the task-state segment itself. So the boundary comes first, on
the simplest mechanism that can hold it, and `syscall` becomes an optimisation
to compare on hardware.

Only **one** gate admits ring 3, `0x80`, and it is named rather than defaulted.
Every other vector raises a general-protection fault when user code says `int`,
because a device's line is not a service.

**`enter_user` is safe because it checks.** `iretq` may land on an unmapped
address in a privilege level that cannot report it, so every precondition is
walked first. The tables are live; the entry is present, `USER` and executable;
the stack is present, `USER`, writable and *not* executable; the stack pointer
is sixteen-byte aligned; and `RSP0` names somewhere for the trap to land. A
caller that gets one wrong is told. Breaking the code page's `USER` bit produces
*"0x400000000000 is not executable from ring 3"* and a red check, where without
the walk it would triple fault.

The frame it pushes sets the interrupt flag, because a ring-3 thread that cannot
be preempted owns the machine. Every general register is zeroed on the way out,
so no kernel pointer reaches user code.

**The descriptor order is fixed now.** `USER_DATA` sits *before* `USER_CODE`,
which looks backwards and is what `sysret` requires: it derives `CS` from
`STAR + 16` and `SS` from `STAR + 8`. Nothing uses `sysret` yet, but moving a
descriptor later means moving every selector that names it.

**Rationale.** The check reads the code selector the *CPU* pushed when it took
the interrupt. A kernel thread saying `int 0x80` cannot manufacture a ring-3
value there, because the number is the hardware's account of where the caller
was. It also checks that the stack selector is ring 3, the stack pointer is in
the low half the kernel never maps, the vector is the one gate, and no other
gate has DPL 3.

When the gate is closed, the machine says so precisely:
`general protection fault (vector 13, error 0x402) at 0x400000000000, cs 0x33, rsp 0x300000001000`.
Error `0x402` is IDT entry `0x80` refused; `cs 0x33` is selector 6 at RPL 3.

**The first program is four bytes and not an ELF.** It is `int 0x80` then
`jmp .`. Slice 7 brings `libs/elf64`, an initrd and `spawn`; this is the layer
beneath, so a failure here is a failure of the boundary, not of a parser. The
trailing jump is unreachable, since the caller is retired from the far side, but
a program falling off its last instruction executes whatever the page holds.

**`retire_from_trap` is `park`'s trap-side twin.** A thread that has crossed
into ring 3 cannot `park`: that wants a `ThreadContext`, and trap context can
never mint one (D-62). So the syscall handler marks the caller blocked with
nothing that will wake it and switches away at trap exit, as preemption does.

**Alternatives.** `syscall`/`sysret` first (rejected: it needs `swapgs` and a
per-CPU block, two unproven mechanisms in the commit that establishes the
privilege boundary, and the measurement that would justify them cannot be taken
here). A gate at DPL 0 with the kernel entering ring 3 some other way (rejected:
there is no other way back). Making `enter_user` `unsafe` (rejected: `kernel`
may not contain an unsafe block (D-13), so the checks must be inside or the
boundary cannot be crossed from where it is needed).

**Consequences.**

- `Frame::write_at` and `Segment::write_at` are the one way safe code puts
  chosen bytes in a frame. They exist so a page can be filled *before* it is
  mapped read-only and executable, never mapped writable and demoted: W^X holds
  for user memory, and it is why these methods exist.
  Slice 7's loader is their second caller.
- **One thread may be in ring 3 at a time.** `RSP0` is set once, to its own
  stack, not the calling thread's. Per-thread `RSP0`, updated by the scheduler
  on every switch, arrives with the processes that need it in slice 7. This is a
  limitation, not a design.
- The retired thread keeps its slot and two mapped pages for the boot's life,
  until `spawn`'s counterpart reclaims retired threads.
- `mod user` is `#[cfg(feature = "selftest")]`, like `handle` and `object`:
  built and proven, and nothing ships it until there is a program to run.
- Ninety-seven named machine checks. The break that goes red is the code page
  losing its `USER` bit. The break that goes *bang* is closing the gate, and its
  fault is better evidence than any assertion.
