# D-96 Ring 3 arrives through a gate of its own, and `enter_user` checks rather than trusts

Status: Accepted (2026-08-24) (its one-thread limit lifted by D-99; corrected and hardened by D-101)

**Context.** D-81 shaped the entry path and left one question open on purpose:
whether the synchronous call set arrives by `syscall`/`sysret` or a trap gate.
Its reasoning was that `sysret`'s non-canonical `RCX` hazard is a real CVE class
costing a branch on every return, a trap gate costs more per call, *and the
measurement that would settle it is meaningless under TCG* (`010` §8).

**Decision. An interrupt gate at DPL 3, and the reason is not performance.**
(Called "a trap gate" throughout the first version of this entry, which was
wrong twice over: the mechanism chosen is an *interrupt* gate — the flag is
cleared on entry — and `libs/x86-tables` says a trap gate "is deliberately
unavailable". Corrected by D-101.) `syscall` needs
`swapgs`, `swapgs` needs a per-CPU block, and a per-CPU block reached through
`gs:` is a second unproven mechanism in the same commit as the privilege
boundary. A gate reached through the interrupt table needs none of it: the CPU
loads `RSP0` from the task-state segment itself. So the boundary is established first, on the
simplest mechanism that can hold it, and `syscall` becomes an optimisation with
something working to compare against — on hardware, where the comparison means
something.

Exactly **one** gate admits ring 3, `0x80`, and it is named rather than
defaulted. Every other vector raises a general-protection fault when user code
says `int`, because a device's line is not a service.

**`enter_user` is safe because it checks.** `iretq` will send the CPU to an
unmapped address in a privilege level that cannot report it, so every
precondition is walked first: the tables are live; the entry is present, `USER`
and executable; the stack is present, `USER`, writable and *not* executable;
the stack pointer is sixteen-byte aligned; and `RSP0` names somewhere for the
trap to land. A caller that gets any of them wrong is told. Breaking the code
page's `USER` bit produces *"0x400000000000 is not executable from ring 3"* and
a red check, where without the walk it would produce a triple fault.

The frame it pushes sets the interrupt flag, because a ring-3 thread that
cannot be preempted owns the machine. Every general register is zeroed on the
way out: a register left set is a kernel pointer handed to user code.

**The descriptor order is fixed now.** `USER_DATA` sits *before* `USER_CODE`,
which looks backwards and is what `sysret` requires — it derives `CS` from
`STAR + 16` and `SS` from `STAR + 8`. Nothing uses `sysret`, and the order is
settled anyway, because moving a descriptor later means moving every selector
that names it.

**Rationale.** The evidence this produces is the point. The check reads the
code selector the *CPU* pushed when it took the interrupt — a ring-3 value
there cannot be manufactured by a kernel thread saying `int 0x80`, because the
number is the hardware's account of where the caller was. Alongside it: the
stack selector is ring 3, the stack pointer is in the low half the kernel never
maps, the vector is the one gate, and no other gate has DPL 3.

And when the gate is closed, the machine says so precisely:
`general protection fault (vector 13, error 0x402) at 0x400000000000, cs 0x33,
rsp 0x300000001000`. Error `0x402` is IDT entry `0x80` refused; `cs 0x33` is
selector 6 at RPL 3. The hardware confirming every part of the boundary at
once.

**The first program is four bytes and deliberately not an ELF.** `int 0x80`
then `jmp .`. Slice 7 brings `libs/elf64`, an initrd and `spawn`; this is the
layer beneath, and keeping them apart means a failure here is a failure of the
boundary rather than of a parser. The trailing jump is unreachable — the caller
is retired from the far side — but a program whose last instruction falls
forwards executes whatever the page holds, and stopping deliberately is cheaper
than reasoning about that.

**`retire_from_trap` is `park`'s trap-side twin.** A thread that has crossed
into ring 3 cannot `park`: that wants a `ThreadContext`, and trap context
cannot mint one by construction (D-62). So the syscall handler marks the caller
blocked with nothing that will ever wake it and switches away at trap exit,
exactly as preemption does.

**Alternatives.** `syscall`/`sysret` first (rejected: it needs `swapgs` and a
per-CPU block, which is two unproven mechanisms in the commit that establishes
the privilege boundary — and the measurement that would justify the complexity
cannot be taken here); a gate at DPL 0 with the kernel entering ring 3 some
other way (rejected: there is no other way back); making `enter_user` `unsafe`
(rejected: `kernel` may not contain an unsafe block (D-13), so the checks have
to be inside or the boundary cannot be crossed from where it is needed).

**Consequences.**

- `Frame::write_at` and `Segment::write_at` are the one way safe code puts
  chosen bytes in a frame. They exist so a page can be filled *before* it is
  mapped read-only and executable, rather than mapped writable and demoted —
  W^X is not relaxed for user memory, it is the reason user memory exists.
  Slice 7's loader is their second caller.
- **One thread may be in ring 3 at a time.** `RSP0` is set once, to a stack of
  its own, rather than to the calling thread's. Per-thread `RSP0`, updated by
  the scheduler on every switch, arrives with the processes that need it in
  slice 7. This is a limitation, not a design.
- The retired thread keeps its slot and its two mapped pages for the life of
  the boot. Nothing reclaims a retired thread yet; `spawn`'s counterpart is
  what will.
- `mod user` is `#[cfg(feature = "selftest")]`, like `handle` and `object`:
  built and proven, and nothing ships it until there is a program to run.
- Ninety-seven named machine checks. The break that goes red is the code page
  losing its `USER` bit; the break that goes *bang* is closing the gate, and
  the fault it produces is better evidence than any assertion.
