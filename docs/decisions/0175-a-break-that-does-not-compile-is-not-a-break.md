# D-175 A break that does not compile is not a break

Status: Accepted, 2026-08-25.

**Context.** The Phase 6 close ran the full sweep and one of sixty-four
mutations did not come back caught: *a device window is uncacheable*. The
summary line said **survived**, which means "a property nothing checks" — the
most alarming thing that task can say.

It was not that. The per-mutation line, five thousand lines earlier in the log,
said `inconclusive: kernel build failed`. The mutation's replacement was
`Flags::empty()`, and `Flags` has no such function: it has `NONE`. The mutation
had never run.

**Two failures, and the second is mine rather than the harness's.**

**The summary conflated two different things.** `run` already kept a build
failure apart from a survivor in its reasoning — the comment says so
explicitly, "a build failure is not a survivor" — and then pushed it onto the
same `survivors` list and reported it with the same word. A reader who sees
"survived" goes looking for a missing check. There is no missing check.

**The break that introduced it had the same defect and hid it.** Slice 5's
break script wrote `Flags::empty()`, booted, and reported red because the boot
produced no `verdict v1 PASS` line. A build failure produces no PASS line
either. So the break was recorded as red, the property was recorded as
verified, and it was not: until this sweep, nothing had ever shown that
`the controller's registers are uncacheable` can fail.

**Decision.** Three things.

`Flags::NONE` is the mutation, and it builds. Measured directly rather than
argued: the device window's entry is `0x8000_0000_0000_0073` normally and
`0x8000_0000_0000_0063` under the mutation — bit 4, `PCD`, is exactly what
disappears, and exactly what the check reads.

**The sweep reports a mutation that will not build as itself.** Separate list,
separate line, separate message: *"did not build … each is a defect in the
mutation, not a missing check"*. The two want different work, and naming them
alike sends the reader to the wrong one.

**The rule, which is what this entry is for:**

> **A break is only red if the thing that went red is a check.** Absence of a
> pass is not a failure — a build error, a harness crash, a timeout and a red
> check all produce no `PASS`, and only one of them is evidence. A break script
> that greps for the absence of success is measuring the wrong thing.

**Rationale.** D-28's whole argument is that a check which cannot fail is worth
less than no check, *because it is also a claim*. A break that cannot run is
the same defect one level out: it produces the appearance of having verified
something, and it is the appearance that does the damage — the property goes on
the record as checked and nobody looks again.

The reason this was caught at all is that the mutation sweep exists. Breaking a
check by hand happens once, when it is written; `cargo xtask mutate` is D-119's
answer to *that* being unrepeatable, and here it caught not a rotted check but a
rotted **break** — a claim made in a commit message six slices ago that was not
true when it was made.

**Consequences.**

- One mutation corrected. The property it tests is now genuinely verified,
  measured at the page-table entry rather than inferred from a boot's exit.
- `cargo xtask mutate` distinguishes survivors from mutations that would not
  build, in the summary as well as in the per-mutation line.
- Slice 5's break record is amended rather than left standing: *"a register
  file the cache may answer for"* was reported red and was not. `014`'s slice 5
  section says so.
- `005` carries the rule beside D-28's.
- Every other break in this phase used an edit that compiled — the sweep's
  sixty-three catches are what says so, since a mutation that does not build is
  now impossible to mistake for one that does.
