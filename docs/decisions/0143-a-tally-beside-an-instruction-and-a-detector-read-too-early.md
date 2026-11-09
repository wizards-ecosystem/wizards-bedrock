# D-143 A tally beside an instruction, and a detector read too early

Status: Accepted (2026-08-25)

**Context.** Two checks that existed, passed, and could not fail — both about
mechanisms this kernel's security rests on.

**The buffer flush counted its own neighbour.** `VERW` on the return to ring 3
is the MDS mitigation, and D-101 already cost this project a phase by shipping
it unexecuted: reading the *arming* byte says the instruction was compiled in,
not that it ran. D-105's fix was to count in the assembly, "beside the `verw`
itself".

Beside is not the same as because. The stub was:

```
verw word ptr [rsp]
add rsp, 8
inc qword ptr [rip + {clears}]
```

Delete the `verw` line alone and the `inc` still runs, the tally still climbs,
`buffer_clear_is_armed()` is still true, and the check is green with the
mitigation gone. D-101's lesson had been learned one instruction short of
where it needed to be.

**The leaf-nesting detector was read at check eighty of a hundred and
forty-three.** D-126's counter records any lock taken while a `lock_leaf` guard
is held and keeps the **first** such violation. `taken_under_a_leaf()` has
exactly one reader, and it ran two-thirds of the way through the run — so a
violation in any of the sixty checks after it, over `input.rs`'s twelve
`lock_leaf` sites and `dev.rs`'s three, happened after the only read and was
never reported.

**Decision.**

`verw` sets ZF when the selector is writable at the current privilege level,
which the kernel data segment is. The tally is now gated on that flag:

```
verw word ptr [rsp]
jnz 3f
inc qword ptr [rip + {clears}]
3:
add rsp, 8
```

With the `verw` deleted, ZF is whatever `sub rsp, 8` left — clear — so the
increment does not run and the check goes red. The count is now of the
instruction rather than of its position. Both stubs, the trap exit and the
first crossing, carry it.

The detector's read moves to the end of the run, beside the console-scroll
check, with a comment saying why each must be last: one reads a value every
other check can change, the other writes over what every other console check
looks at.

**Moving it found a violation immediately, which is the best evidence it was
in the wrong place.** `a_lock_excludes_interrupts` took two `lock_leaf` guards
deliberately, to check that dropping the inner one does not re-enable
interrupts. The property is real and worth checking; the nesting was the one
direction `lock_leaf` forbids, and it would have masked every later genuine
violation, because the detector keeps the first.

The check now takes the **token-bearing** form for the outer guard and the leaf
for the inner — the sanctioned direction, the one that makes D-47's
`HEAP` → `FRAMES` order sound — which is what it needed anyway: the property
under test is about interrupts, not about lock order.

**Rationale.** Both defects are the same mistake at different scales:
**position mistaken for participation.** An `inc` next to a `verw` is not
counting the `verw`; a detector read before sixty checks is not watching them.
In each case the check was written by someone who had the right idea and
stopped one step short of making the mechanism carry it.

The `verw` fix is worth the awkwardness of a conditional in a hot path — two
extra instructions on the exit to ring 3, one of them not taken — because the
alternative is a mitigation whose only evidence is a number that would be
identical without it. D-101's original defect and this one are the same
sentence with different subjects.

**Consequences.**

- The flush's tally is causal. Deleting either `verw` turns
  `the_return_to_ring_three_clears_buffers` red; it did not before.
- The leaf detector watches the whole run. Its first read found a real
  violation in the checks themselves.
- `a_lock_excludes_interrupts` uses the sanctioned nesting. It tests the same
  property and no longer commits the thing another check is looking for.

**Left owed.** The detector still keeps only the *first* violation, so a
deliberate one anywhere still masks the rest. Nothing needs it today — there
are none — and the honest fix is a small ring of the last few, which is worth
doing when there is a second CPU and the counter has to move per-CPU anyway
(D-126's owed item).

The audit's remaining Tier 2 findings include two more of this family: the
`xstate` save form is dispatched on a flag no check reads, and
`ring_proto::is_consistent` is asserted true at every call site and never
false. Neither is closed here.
