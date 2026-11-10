# D-119 Breaking the checks is a task, not a habit

Status: Accepted, 2026-08-24.

**Context.** D-28 is the rule this project takes most seriously: every new check
is broken once and watched go red, because a check that cannot fail is worth
less than no check. It is also a claim. Five adversarial audits have now found
roughly twenty checks that could not fail, several in the same commit as an ADR
asserting they had been broken.

The rule stands; **doing it by hand fails.** A break-test happens once, when the
check is written, and never again. A check that *stops* being able to fail
because the code around it changed is invisible from then on. No amount of
discipline fixes that, so it is worth automating.

**Decision.** `cargo xtask mutate`. Each entry is a defect, such as a bound
removed, a flag not stripped or a guard inverted. Applying one and booting must
turn some check **red**. A mutation that survives is a property nothing checks,
and the report names it.

Ten to begin with, each removing something else depends on, none a cosmetic edit
that would fail the build instead. A mutation that cannot compile has not been
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

**The first run found a defect of the same class in itself.** The mutation meant
to test "a boot mapping cannot launder a frame into the allocator" targeted the
flag strip in `map_run`. That function adds `OWNED` back on the next line.
Stripping the caller's flag changed nothing, and the mutation survived. The
property was checked all along; the *mutation* was not testing it. A
hand-written break-test makes the same mistake and never reports it.

Retargeted at `map_boot`'s own strip, all ten are caught.

**Consequences.** The next audit starts from a shorter list. These ten
properties are now checked *and shown to be checkable*, continuously, not at one
past moment. Adding a mutation is the natural companion to adding a check, and
the gate notices when one rots.

Left owed: ten mutations is a sample, not a sweep. The obvious growth is one
mutation per invariant an ADR claims, so the machine says "this decision is
enforced" instead of a document. Nothing yet mutates the `libs/` crates, where
`cargo test` is fast enough that a full sweep takes seconds, not minutes.
