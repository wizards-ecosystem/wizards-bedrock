# D-119 Breaking the checks is a task, not a habit

Status: Accepted, 2026-08-24.

**Context.** D-28 is the rule this project takes most seriously: every new check
is broken once and watched go red, because a check that cannot fail is worth
less than no check — it is also a claim. Five adversarial audits have now found
roughly twenty checks that could not fail, several of them in the same commit
as an ADR asserting they had been broken.

The rule is not wrong. **Doing it by hand is.** A break-test happens once, at
the moment the check is written, and never again — so a check that *stops*
being able to fail, because the code around it changed, is invisible from then
on. That is not a discipline problem; it is a problem no amount of discipline
fixes, which is what makes it worth automating.

**Decision.** `cargo xtask mutate`. Each entry is a real defect — a bound
removed, a flag not stripped, a guard inverted — and applying one and booting
must turn some check **red**. A mutation that survives is a property nothing
checks, and the report names it.

Ten to begin with, chosen to be *load-bearing rather than merely different*:
each removes something something else depends on, and none is a cosmetic edit
that would fail the build instead. A mutation that cannot compile has not been
tested by anything, so a build failure is reported as **inconclusive and
counted as a survivor** — reporting it as caught would be the same lie this
task exists to find.

**Two tiers, and the cheap one matters more.** `cargo test` requires every
mutation's pattern to occur **exactly once** in its file — no boot, no build,
milliseconds, and therefore in the gate. A pattern that has stopped matching is
a mutation that silently stopped testing anything, which is precisely the
failure mode this file exists to catch, one level up. Zero matches and two
matches are both errors: with two, which one it breaks is not decided.

The expensive tier is one QEMU boot per mutation and is **not in `ci`**, for
the reason `model-check` and `vmware` are not: a gate step that costs ten
minutes is a gate people learn to skip.

**Restoring is not best-effort.** The original is held in memory, written back
*before* anything is allowed to fail — including the check on whether the
mutation was caught — so no error path can leave the tree modified.

**The first run found a defect in itself, which is the same class of finding
and exactly as welcome.** The mutation for "a boot mapping cannot launder a
frame into the allocator" targeted the flag strip in `map_run`, which adds
`OWNED` back on the next line — so stripping the caller's changed nothing and
the mutation survived. The property was checked all along; the *mutation* was
not testing it. A hand-written break-test makes that mistake exactly as easily
and never reports it.

Retargeted at `map_boot`'s own strip, all ten are caught.

**Consequences.** The next audit starts from a shorter list: these ten
properties are now checked *and demonstrated to be checkable*, continuously,
rather than at one moment in the past. Adding a mutation is the natural
companion to adding a check, and the gate will notice when one rots.

Left owed: ten mutations is a sample, not a sweep. The obvious growth is one
mutation per invariant an ADR claims — which would make "this decision is
enforced" a thing the machine says rather than a thing a document asserts. And
nothing yet mutates the `libs/` crates, where `cargo test` is fast enough that
a full sweep is seconds rather than minutes.
