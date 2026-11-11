# D-119 Breaking the checks is a task, not a habit

Status: Accepted, 2026-08-24.

**Decision.** `cargo xtask mutate`. Each entry is a defect, such as a bound
removed, a flag not stripped or a guard inverted. Applying one and booting must
turn some check **red**. A mutation that survives is a property nothing checks,
and the report names it.

Each mutation removes something else depends on, never a cosmetic edit that
would fail the build instead. A mutation that cannot compile has not been
tested by anything, so a build failure is reported as **inconclusive and counted
as a survivor**. Reporting it as caught would hide the failure this task exists
to find.

**Two tiers, and the cheap one matters more.** `cargo test` requires every
mutation's pattern to occur **exactly once** in its file. That needs no boot and
no build and takes milliseconds, so it is in the gate. A pattern that stops
matching is a mutation that silently stopped testing anything, the same failure
one level up. Zero matches and two matches are both errors: with two, which one
it breaks is undecided.

The expensive tier is one QEMU boot per mutation and is **not in `ci`**, for the
reason `model-check` and `vmware` are not: people learn to skip a gate step that
costs ten minutes.

**Restoring is not best-effort.** The original is held in memory and written
back *before* anything may fail, including the check on whether the mutation was
caught, so no error path can leave the tree modified.

**Rationale.** D-28 stands: every new check is broken once and watched go red.
**Doing it by hand fails**, because a break-test happens once, when the check is
written, and a check that *stops* being able to fail because the code around it
changed is invisible from then on. The first run proved the point on itself: a
mutation of `map_run`'s flag strip survived because the next line adds `OWNED`
back, so it was retargeted at `map_boot`'s own strip. A hand-written
break-test makes the same mistake and never reports it.

**Consequences.** Mutated properties are checked *and shown to be checkable*,
continuously, not at one past moment. Adding a mutation is the natural companion
to adding a check, and the gate notices when one rots.

Left owed: the first mutations are a sample, not a sweep. The obvious growth is
one mutation per invariant an ADR claims, so the machine says "this decision is
enforced" instead of a document. Nothing yet mutates the `libs/` crates, where
`cargo test` is fast enough that a full sweep takes seconds, not minutes.
