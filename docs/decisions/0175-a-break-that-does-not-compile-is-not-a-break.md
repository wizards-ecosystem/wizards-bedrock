# D-175 A break that does not compile is not a break

Status: Accepted, 2026-08-25.

**Decision.**

> **A break is only red if the thing that went red is a check.** Absence of a
> pass is not a failure — a build error, a harness crash, a timeout and a red
> check all produce no `PASS`, and only one of them is evidence. A break script
> that greps for the absence of success is measuring the wrong thing.

**The sweep reports a mutation that will not build as itself**, on a separate
list, line and message: *"did not build … each is a defect in the mutation, not
a missing check"*. A survivor sends the reader looking for a missing check; a
build failure is a defect in the mutation, and naming them alike sends the
reader to the wrong work.

The Phase 6 close found *a device window is uncacheable* reported as survived
when its replacement, `Flags::empty()`, did not exist and the mutation had never
run; slice 5's hand break had recorded the same property red from a missing
PASS line. The mutation is `Flags::NONE`, and it builds. Measured at the
page-table entry: the device window is `0x8000_0000_0000_0073` normally and
`0x8000_0000_0000_0063` under the mutation, so bit 4, `PCD`, is exactly what
disappears and exactly what the check reads.

**Rationale.** D-28's argument is that a check which cannot fail is worth less
than no check, because it is also a claim. A break that cannot run is the same
defect one level out: it produces the appearance of having verified something,
and the appearance does the damage, because the property goes on the record as
checked and nobody looks again. The mutation sweep (D-119) is what makes a
rotted break findable.

**Consequences.**

- Slice 5's break record is amended rather than left standing: *"a register
  file the cache may answer for"* was reported red and was not. `014`'s slice 5
  section says so.
- `005` carries the rule beside D-28's.
