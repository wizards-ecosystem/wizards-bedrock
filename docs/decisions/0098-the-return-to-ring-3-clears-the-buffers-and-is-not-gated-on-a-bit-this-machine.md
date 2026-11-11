# D-98 The return to ring 3 clears the buffers, and is not gated on a bit this machine cannot read

Status: Accepted, 2026-08-24.

**Decision.** The `VERW` flush on exit-to-user is armed **unless** the
processor reports `MDS_NO`. D-81 specified it *"gated on `MD_CLEAR`"*, but
QEMU's TCG reports no `IA32_ARCH_CAPABILITIES` (D-91) and no `MD_CLEAR`, so
that gate would leave the instruction never executed on the development loop.

`verw` exists on every x86-64; `MD_CLEAR` says only whether it *also* clears
the microarchitectural buffers. A machine whose capability word cannot be read
gets the instruction, not the assumption: a few cycles on a return, against
skipping a flush a machine needed. `MDS_NO` is the single claim that makes it
pointless, and it is honoured.

The branch is in the entry stub's tail, after the frame is restored. The saved
`CS` is tested for ring 3, and the flush is skipped for a return to ring 0. The
armed flag is a plain byte, read with one instruction and written once at boot,
before any user code exists.

**Rationale.** This policy makes the flush checkable. Under D-81's gate the
instruction would have shipped unexecuted, verified by reading; under this one
it runs on every ring-3 return the self-tests take. No check can show that the
flush flushes. The check shows that the branch is armed and that a return went
through it: the part that could crash, and the part a later edit could silently
remove.

**Alternatives.** Gate on `MD_CLEAR` as D-81 specified (rejected: unreachable
here, and if the gate is wrong the mitigation is silently absent). Flush
unconditionally including returns to ring 0 (rejected: it is every return this
kernel makes, for no boundary crossed). Use the software sequence Linux falls
back to without `MD_CLEAR` (rejected: it is long, processor-specific, and
nothing here can measure whether it helps).

**Consequences.**

- The boot log names both the decision and its limit: *"buffers cleared on
  every return to ring 3 (md_clear unreported, so the flush may buy nothing
  here)"*. A machine reporting `MDS_NO` says so instead.
- D-81's second one-liner, eIBRS, stays armed but unexecutable here (D-91).
  `VERW`'s policy could move off the unreadable bit; eIBRS's cannot, because
  enhanced IBRS needs the capability word to know it exists.
