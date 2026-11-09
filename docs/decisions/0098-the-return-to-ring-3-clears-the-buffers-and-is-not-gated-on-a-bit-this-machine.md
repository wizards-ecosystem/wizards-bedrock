# D-98 The return to ring 3 clears the buffers, and is not gated on a bit this machine cannot read

Status: Accepted (2026-08-24)

**Context.** D-81 listed `VERW` on exit-to-user as one of two one-liners worth
taking before ring 3 exists, *"gated on `MD_CLEAR`"*, on the grounds that an
instruction on a mature return path is painful to retrofit. Ring 3 exists now
(D-96), so the path exists to put it on.

**The gate D-81 specified would make it unreachable here.** QEMU's TCG reports
no `IA32_ARCH_CAPABILITIES` (D-91) and no `MD_CLEAR` — asked directly, with
`-cpu max,+md-clear` and `-cpu Skylake-Server-v4,+md-clear`. Gating on that bit
would have left the instruction never executed on the development loop, in the
same untestable state eIBRS is still in.

**Decision.** The flush is armed **unless** the processor reports `MDS_NO`.

`verw` exists on every x86-64 — it is an old instruction that verifies a
segment for writing — and `MD_CLEAR` says only whether it *also* clears the
microarchitectural buffers. So a machine whose capability word cannot be read
gets the instruction rather than the assumption, which is the direction to be
wrong in: the cost is a few cycles on a return, and the alternative is skipping
a flush on a machine that needed one. `MDS_NO` is the single claim that makes
it pointless, and it is honoured.

The branch is in the entry stub's tail, after the frame is restored: the saved
`CS` is tested for ring 3, and the flush is skipped for a return to ring 0 —
which is every return this kernel makes except the ones that matter. The armed
flag is a plain byte, read with one instruction and written once at boot,
before any user code exists.

**Rationale.** The policy is what makes this checkable at all, and that is the
whole argument. Under D-81's gate the instruction would have been shipped
unexecuted, verified by reading; under this one it runs on every ring-3 return
the self-tests take, and the ring-3 crossing check passes *through it*.

What no check here can show is that the flush flushes — nothing observable
changes, and on this machine it very likely buys nothing, which the boot log
says out loud. What it shows is that the branch is armed and that a return was
taken through it, which is the part that could crash and the part a later edit
could silently remove.

**Alternatives.** Gate on `MD_CLEAR` as D-81 specified (rejected: unreachable
here, and the failure mode of the gate being wrong is the mitigation silently
absent); flush unconditionally including returns to ring 0 (rejected: it is
every return this kernel makes, for no boundary crossed); use the software
sequence Linux falls back to without `MD_CLEAR` (rejected: it is long, it is
processor-specific, and nothing here can measure whether it helps).

**Consequences.**

- Ninety-nine named machine checks. The break is to leave it disarmed.
- The boot log names both the decision and its limit: *"buffers cleared on
  every return to ring 3 (md_clear unreported, so the flush may buy nothing
  here)"*. A machine reporting `MDS_NO` says so instead.
- D-81's second one-liner, eIBRS, remains armed-but-unexecutable here (D-91).
  The difference is that `VERW` had a policy that could be moved off the
  unreadable bit and eIBRS does not: enhanced IBRS genuinely requires the
  capability word to know it exists.
