# D-338 Extended-state admission uses one exact live policy and a fresh initial image

Status: Accepted (2026-09-28), amends D-82, D-115, D-228, D-270 and D-333; closes PW-132

**Context.** The extended-state implementation already required x87, SSE and
AVX, sized standard XSAVE from reported component extents, cleared every fresh
task area and checked live isolation at both user doors. PW-132 remained open
because those independent policy, image and admission edges had not all been
made causally red against the current 262-check machine. VMware's compacted
`XSAVEC` execution remains PW-305; it is a separate hardware-form
qualification, not missing local evidence for these invariants.

**Decision.** Processor availability must contain every component in the fixed
x87/SSE/AVX policy, and the live XCR0 configuration must equal that policy
exactly. Standard-form size includes each present component at its reported
offset and rejects any overlap with the legacy/header region or arithmetic
overflow.

A new or recycled task slot contains the architectural all-zero initial image
across its complete aligned XSAVE ceiling before its first restore. That image
is valid independently of any previous save. Early process construction and
the final ring-three crossing both ask the same live predicate: configured
form, live `CR4.OSXSAVE` and exact live XCR0 must all agree at the point of
use.

**Evidence.** On 2026-09-28 retained mutations 431, 432 and 717 were applied
individually and restored byte-exact. Admitting any partial permitted mask
failed only `xstate::tests::every_permitted_component_must_be_available`;
accepting a standard component below the header failed only
`xstate::tests::a_standard_component_extent_uses_the_reported_offset`; and
admitting extra live XCR0 state failed only
`xstate::tests::the_live_configuration_is_exactly_the_fixed_policy`. Each
exact run failed one test with 32 filtered out.

Release QEMU mutation 310 bypassed the early live-isolation refusal, while
mutation 1,170 substituted remembered enablement for that predicate. Each
failed only `missing extended state closes the user boundary` at 261/262.
Mutations 828 and 829 independently skipped the complete task-area clear and
disabled the witness's secret write; each failed only `an exited slot is
reused` at 261/262. D-333 records the independent final-door mutation 1,171,
which replaced only that production predicate and failed only the same boundary
check at 261/262 before exact restoration passed 262/262.

After every current break was restored, all 33 `x86-cpuid` unit tests and three
doctests passed, release QEMU passed 262/262, and applicability plus named-check
registration passed for all 1,412 retained mutations.

**Alternatives.** Treating any nonempty permitted subset as sufficient was
rejected because code may use every fixed component. Allowing extra live XCR0
bits was rejected because the allocated image does not promise their layout.
Saving a task once merely to initialize it was rejected because architectural
zeros already encode the defined initial state. Remembered enablement was
rejected at either user door because only current control-register state can
authorize the crossing.
