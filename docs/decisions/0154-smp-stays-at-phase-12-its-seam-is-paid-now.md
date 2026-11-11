# D-154 SMP stays at Phase 12; its seam is paid now

Status: Accepted, 2026-08-25.

**Decision.** SMP stays where the roadmap put it, at Phase 12, and the
**per-CPU seam** is made real now: state that must become per-CPU moves into
`arch::percpu` while there is one CPU and the move is mechanical. The module
holds one `PerCpu` block behind an accessor every caller goes through. On one
processor the accessor returns the single instance; when an AP starts, the
accessor reads `gs:` and nothing else in the tree changes.

**Rationale.** Retrofitting SMP is the classic disaster, but the expensive half
already exists: the lock ranks, the leaf discipline and the borrow token (D-105,
D-121, D-126, D-70) exist because D-56 and D-70 asked for them in Phase 5. An OS
that can schedule on two CPUs and still cannot open a file is worse than one
that can open a file, so storage comes first. Paying the seam now, rather than
changing nothing, stops the debts accumulating: every static that will need to
be per-CPU is cheaper to move today than after the network stack adds more. The
races stay unreproducible; the structure stops being owed.

**Corrected.** D-81 wrote that `arch::percpu` "has to exist for ring 3
regardless", and an earlier draft of this entry repeated it. It did not exist:
ring 3 went through an interrupt gate rather than `syscall` (D-96), so `swapgs`
was never needed. That is D-140's defect class, a `SAFETY` comment naming the
wrong reason passing review as well as one naming the right one.

**Consequences.**

- `arch::percpu` exists, holding `spinlocks_held`, `leaves_held` and the
  leaf-violation record, and every caller goes through it. D-126's owed move of
  `LEAVES_HELD` is discharged structurally.
- The `gs:`-based accessor is not built, because there is nothing to point it
  at. The seam is the module boundary, not the segment register.
- The races that need a second CPU stay recorded and unreproducible: D-129's
  two-caller race in `pic::silence`, and D-141's and D-145's interleavings in
  the handle arena and `receive_blocking`. None of them is reachable.
- Phase 12 inherits a smaller job: bring up an AP, with its state already where
  it needs to be.
- The honest test for an unreproducible interleaving is a stress check with a
  flake budget, and this project has none and does not want one. Whoever brings
  up the second CPU decides what that costs; it is named in D-141 and D-145 and
  not decided here.
