# D-134 A mutation is caught when *its* check goes red, not when the boot does

Status: Accepted (2026-08-25)

**Context.** D-119 made breaking the checks a task: apply a known defect, boot,
require something to go red. The runner asked `boot_is_red` — whether the
verdict was `FAIL`. Not which check failed. `Mutation::breaks` names the
property, and it was printed in a progress line and compared to nothing.

So "caught" meant *some* check went red. A peer session auditing the table
found the gap, and found it live rather than hypothetical:

- **Mutation 4** — `a reclaimed frame is cleared before the kernel sees it` —
  reddens two checks: `a user page comes back cleared`, its own target, and
  `kernel memory cannot be published to ring three`, which is **mutation 3's**.
  Delete mutation 4's target tomorrow and it goes on printing "caught" for
  ever, off the back of another entry's check.
- **Mutations 2 and 10** overlap on ten checks between them. Either could lose
  its own target and never say so.
- Mutation 2 reddens thirteen checks and mutation 10 eleven. Neither is wrong;
  "caught" tells you almost nothing about either.

This is the sentence in `mutate.rs`'s own module doc — *"a mutation that
silently stopped testing anything, which is precisely the failure mode this
whole file exists to catch"* — applied to the expensive tier, which the file
built the cheap tier to protect it from. It is also D-124's shape exactly: a
check measuring a description rather than the thing.

**It made a published claim false.** The sweep run earlier the same day was
reported as "seventeen mutations, seventeen caught", meaning each defect broke
the property it names. What the tool established was that seventeen boots went
red. The weaker statement was the true one, and the gap was a shortcut in the
runner.

**Options.**

1. **Record the limit** in D-119 and move on. Cheap, honest, and leaves the
   sweep's output meaning less than it says. Proposed on the grounds that no
   mutation was currently falling through — which mutation 4 refutes.
2. **A cheap tier only**: assert in `cargo test` that each named check exists.
   Milliseconds, at every commit. Catches a renamed or deleted target; cannot
   see which check a boot actually reddened.
3. **Both tiers.**

**Decision.** Option 3, and the pricing that settles it is not
boots-against-a-hypothetical: the twenty boots are the *verification*, and
they upgrade every future sweep's output permanently. The break-test is one
boot, not a second sweep.

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

`breaks` stays, and that is deliberate. **Only six of the twenty are check
names**; the other fourteen are paraphrases, and they read better in a progress
line than the checks they stand for. Collapsing to one field would mean either
fourteen worse progress lines or fourteen renamed checks — a test bending the
code to satisfy itself.

`Caught::Panic` carries its reason because a bare variant is the way to dodge
naming a check. Two entries use it: without the fixup a faulting user copy is
terminal, so the kernel dies before any verdict; and logging from trap context
trips `ThreadLock`'s assert, which is the defect being a defect.

`no_two_mutations_name_the_same_property` still compares `breaks`, not
`reddens`. Mutations 4 and 14 both name `a user page comes back cleared` —
one check, two properties, two defects, because that check asserts both the
reclaim path and the drop tally. Forcing them apart would push someone to split
a good check to satisfy a test.

**The check names come from the machine.** `qemu::SerialObservation` now
records every `selftest: <name> FAILED` line and `correlate_selftest` puts them
in the error, so the runner compares against the kernel's own report rather
than anything it inferred.

**Rationale.** The two tiers catch different things and neither subsumes the
other. The cheap one catches a target that no longer exists — invisible to the
sweep, because the mutation still boots and still reddens something. The
expensive one catches a target that exists, is fine, and *is not the one that
failed* — invisible to any host test, because no amount of string matching
knows which check a boot reddened.

**What the first strict sweep did, and it is the argument for running it.** It
reported **eighteen survivors** on boots that had each reddened exactly the
right check. Serial lines arrive with a trailing `\r`, so `strip_suffix(" FAILED")`
matched nothing and every name list was empty. The fix is one `trim_end`. Had
this been reasoned about rather than run, the table would have shipped with a
comparison that never matched — a stricter-looking check that was strictly
worse, since it would have reported survivors for defects that were caught.
That is the same lesson as D-131's macro measurement, a week's worth of
sessions apart: the tool's behaviour is not deducible from its description.

**Consequences.**

- `cargo xtask mutate` prints *"every mutation was caught by the check it
  names"*, and that sentence is now true. Twenty for twenty at `3df6281`.
- Two host tests: every named check exists in `selftest.rs`, and every `Panic`
  says why. Both broken once and watched go red.
- `parse_failed_check` has its own test, carriage return included, because that
  is the character that cost a twenty-boot sweep.
- Mutation 10's target was **wrong and was corrected by measurement**. It was
  mapped by reasoning to `two spaces map one address to different memory` — and
  that check reads through `translate_in_root`, which walks a root it is handed
  rather than the one `CR3` holds, so it answers the same either way and cannot
  see the defect at all. It is `a second thread crosses onto its own stack`.
- `is_a_verdict` is gone; `boot_reports` distinguishes the two verdicts itself.

**Left owed.** What survives both tiers: a mutation whose named check still
exists, is still named in the log, and has quietly stopped being able to fail —
with the mutation's own check reddening for some *other* reason. Nothing short
of a per-check causal verdict closes that, and nothing here pretends to.

The eleven-and-thirteen-check blast radii of mutations 2 and 10 are not a
defect but are worth knowing: a mutation that reddens a third of the suite is
testing the boot as much as the property, and a narrower defect in the same
place would be a better entry.
