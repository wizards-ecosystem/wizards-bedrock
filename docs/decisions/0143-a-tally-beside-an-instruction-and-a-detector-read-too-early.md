# D-143 A tally beside an instruction, and a detector read too early

Status: Accepted, 2026-08-25.

**Decision.** The `VERW` tally is gated on the instruction's own result. `verw`
sets ZF when the selector is writable at the current privilege level, which the
kernel data segment is:

```
verw word ptr [rsp]
jnz 3f
inc qword ptr [rip + {clears}]
3:
add rsp, 8
```

With the `verw` deleted, ZF is whatever `sub rsp, 8` left (clear), so the
increment does not run and `the_return_to_ring_three_clears_buffers` goes red.
Both stubs, the trap exit and the first crossing, carry it. An `inc` merely
beside the `verw` (D-105's fix) still climbed with the mitigation gone.

D-126's leaf-nesting detector is read at the end of the run, beside the
console-scroll check, with a comment saying why each must be last: one reads a
value every other check can change, the other writes over what every other
console check looks at. Read earlier, a violation in any later check happened
after the only read and was never reported.

`a_lock_excludes_interrupts` takes the **token-bearing** form for the outer
guard and the leaf for the inner, the sanctioned direction. Its two nested
leaves were the direction `lock_leaf` forbids, and the detector, moved, found
them at once.

**Rationale.** **Position mistaken for participation**: an `inc` next to a
`verw` is not counting the `verw`; a detector read before most checks is not
watching them. Two extra instructions on the exit to ring 3, one of them not
taken, are worth it, because the alternative is a mitigation whose only
evidence is a number that would be identical without it.

**Left owed.** The detector still keeps only the *first* violation, so a
deliberate one anywhere still masks the rest. Nothing needs it today, and the
honest fix is a small ring of the last few, which is worth doing when there is
a second CPU and the counter has to move per-CPU anyway (D-126's owed item).

The audit's remaining Tier 2 findings include two more of this family: the
`xstate` save form is dispatched on a flag no check reads, and
`ring_proto::is_consistent` is asserted true at every call site and never
false. Neither is closed here.
