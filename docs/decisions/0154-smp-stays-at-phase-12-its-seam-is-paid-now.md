# D-154 SMP stays at Phase 12; its seam is paid now

Status: Accepted (2026-08-25)

**Context.** Four entries carry debts that cannot be discharged on one CPU:
D-126 owes `LEAVES_HELD` a move into `arch::percpu`; D-129 owes the two-caller
race in `pic::silence`; D-141 and D-145 owe the interleavings in the handle
arena and `receive_blocking`, neither of which can be reproduced without a
second thread inside a window between two lock acquisitions.

Meanwhile the whole lock discipline — D-105's borrow token, D-121's no-closure
rule, D-126's leaf detector, D-70's rules — was **designed** for SMP and has
only ever been **exercised** by one CPU.

**Options.**

1. **Pull SMP forward, before storage.** Exercise the concurrency design at its
   smallest, so every later phase is written against a real second CPU.
2. **Pull it to just after Phase 6.** Storage first, so the machine is useful,
   then a second CPU before the network stack, where concurrency bites hardest.
3. **Leave it at Phase 12 and change nothing.**
4. **Leave it at Phase 12 and pay the seam now.**

**Decision.** Option 4. SMP stays where the roadmap put it. The **per-CPU
seam** is made real now: state that must become per-CPU moves into
`arch::percpu` while there is one CPU and the move is mechanical.

**Rationale.** The argument for pulling SMP forward is real — retrofitting it
is the classic disaster, and four debts wait on it. The argument against is
that this kernel has already done most of the expensive half: the lock ranks, the
leaf discipline and the borrow token all exist because D-56 and D-70 asked for
them in Phase 5.

**`arch::percpu` does not, and an earlier draft of this entry said it did.**
D-81 wrote "`arch::percpu` has to exist for ring 3 regardless" — and it did
not, because ring 3 went through an interrupt gate rather than `syscall`
(D-96), so `swapgs` was never needed and the module was never written. The
claim was corrected before this entry was an hour old, which is the same
defect class D-140 is about and is why that entry says a `SAFETY` comment
naming the wrong reason passes review exactly as well as one naming the right
one.

And an OS that can schedule on two CPUs and still cannot open a file is worse
than one that can open a file. Storage is what makes the machine a machine.

**What makes option 4 different from option 3** is that the debts stop
accumulating. Every static that will need to be per-CPU is cheaper to move
today than after the network stack has three more of them, and moving it is a
mechanical change with no behaviour to get wrong while there is one CPU. The
*races* stay unreproducible; the *structure* stops being owed.

So the seam is **built**, not merely moved into: `arch::percpu` becomes a
module holding one `PerCpu` block, with an accessor every caller goes through.
On one processor the accessor returns the single instance. When an AP starts,
the accessor reads `gs:` and **nothing else in the tree changes** — which is
the entire value, and the reason it is worth doing before there is a second
CPU rather than during.

**Consequences.**

- `arch::percpu` exists, holding `spinlocks_held`, `leaves_held` and the
  leaf-violation record. Every caller goes through it. D-126's owed item is
  discharged structurally.
- The `gs:`-based accessor is **not** built, because there is nothing to point
  it at. The seam is the module boundary, not the segment register.
- The three races remain recorded and unreproducible. Their entries say so, and
  none of them is reachable.
- Phase 12 inherits a smaller job: bring up an AP, and the state it needs is
  already where it needs to be.

**Left owed.** The honest test for an unreproducible interleaving is a stress
check with a flake budget, and this project has no flake budget and does not
want one. Whoever brings up the second CPU decides what that costs; it is named
in D-141 and D-145 and not decided here.
