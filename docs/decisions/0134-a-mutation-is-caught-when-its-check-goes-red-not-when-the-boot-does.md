# D-134 A mutation is caught when *its* check goes red, not when the boot does

Status: Accepted, 2026-08-25.

**Decision.** A mutation is caught only when the check it names goes red.
`Mutation` gains `reddens: Caught`:

```rust
pub enum Caught {
    /// The exact `report.check("…")` name that must appear as `FAILED`.
    Check(&'static str),
    /// Nothing names it: the defect kills the machine before the verdict.
    /// Carries *why*.
    Panic(&'static str),
}
```

`breaks` stays, and that is deliberate: most entries' `breaks` are paraphrases
that read better in a progress line than the checks they stand for, and
collapsing the fields would mean worse progress lines or renamed checks, a test
bending the code to satisfy itself. `Caught::Panic` carries its reason because
a bare variant is the way to dodge naming a check.

`no_two_mutations_name_the_same_property` compares `breaks`, not `reddens`: one
check may assert two properties, and forcing their mutations apart would push
someone to split a good check to satisfy a test.

**The check names come from the machine.** `qemu::SerialObservation` records
every `selftest: <name> FAILED` line and `correlate_selftest` puts them in the
error, so the runner compares against the kernel's own report rather than
anything it inferred. `parse_failed_check` trims the trailing `\r` serial lines
carry, and its test includes one.

**Two tiers.** Host tests require every named check to exist in `selftest.rs`
and every `Panic` to say why; the sweep requires the named check to be the one
that failed.

**Rationale.** D-119's runner asked only whether the verdict was `FAIL`, so a
mutation could lose its own target and go on printing "caught" off the back of
another entry's check. The cheap tier catches a target that no longer exists,
invisible to the sweep; the expensive tier catches a target that exists and *is
not the one that failed*, invisible to any host test. The first strict sweep,
without the `\r` trim, reported survivors for defects that were caught: the
tool's behaviour is not deducible from its description, so it is run.

**Consequences.** `cargo xtask mutate` prints *"every mutation was caught by the
check it names"*, and that sentence is true. A mutation's target is found by
measurement, not reasoning: one mapped by reasoning to a check that walks a
root it is handed could not see the defect at all.

**Left owed.** What survives both tiers: a mutation whose named check still
exists, is still named in the log, and has quietly stopped being able to fail,
with the mutation's own check reddening for some *other* reason. Nothing short
of a per-check causal verdict closes that, and nothing here pretends to.

A mutation that reddens a third of the suite is testing the boot as much as the
property, and a narrower defect in the same place would be a better entry.
