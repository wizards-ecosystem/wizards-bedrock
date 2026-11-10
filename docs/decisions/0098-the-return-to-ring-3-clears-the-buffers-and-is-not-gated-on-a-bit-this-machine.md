# D-98 The return to ring 3 clears the buffers, and is not gated on a bit this machine cannot read

Status: Accepted, 2026-08-24.

**Context.** D-81 listed `VERW` on exit-to-user as one of two one-liners worth
taking before ring 3 exists, *"gated on `MD_CLEAR`"*, because an instruction on
a mature return path is painful to retrofit. Ring 3 exists now (D-96), so the
path exists to put it on.

**The gate D-81 specified would make it unreachable here.** QEMU's TCG reports
no `IA32_ARCH_CAPABILITIES` (D-91) and no `MD_CLEAR`, asked directly with
`-cpu max,+md-clear` and `-cpu Skylake-Server-v4,+md-clear`. Gating on that bit
would leave the instruction never executed on the development loop, untestable
like eIBRS still is.

**Decision.** The flush is armed **unless** the processor reports `MDS_NO`.

`verw` exists on every x86-64: it is an old instruction that verifies a segment
for writing. `MD_CLEAR` says only whether it *also* clears the
microarchitectural buffers. A machine whose capability word cannot be read gets
the instruction, not the assumption: a few cycles on a return, against skipping
a flush a machine needed. `MDS_NO` is the single claim that makes it pointless,
and it is honoured.

The branch is in the entry stub's tail, after the frame is restored. The saved
`CS` is tested for ring 3, and the flush is skipped for a return to ring 0,
which is every return this kernel makes except the ones that matter. The armed
flag is a plain byte, read with one instruction and written once at boot, before
any user code exists.

**Rationale.** This policy makes the flush checkable. Under D-81's gate the
instruction would have shipped unexecuted, verified by reading. Under this one
it runs on every ring-3 return the self-tests take, and the ring-3 crossing
check passes *through it*.

No check here can show that the flush flushes: nothing observable changes, and
the boot log states it likely buys nothing on this machine. The check shows that
the branch is armed and that a return went through it: the part that could
crash, and the part a later edit could silently remove.

**Alternatives.** Gate on `MD_CLEAR` as D-81 specified (rejected: unreachable
here, and if the gate is wrong the mitigation is silently absent). Flush
unconditionally including returns to ring 0 (rejected: it is every return this
kernel makes, for no boundary crossed). Use the software sequence Linux falls
back to without `MD_CLEAR` (rejected: it is long, processor-specific, and
nothing here can measure whether it helps).

**Consequences.**

- Ninety-nine named machine checks. The break leaves it disarmed.
- The boot log names both the decision and its limit: *"buffers cleared on
  every return to ring 3 (md_clear unreported, so the flush may buy nothing
  here)"*. A machine reporting `MDS_NO` says so instead.
- D-81's second one-liner, eIBRS, stays armed but unexecutable here (D-91).
  `VERW`'s policy could move off the unreadable bit; eIBRS's cannot, because
  enhanced IBRS needs the capability word to know it exists.
