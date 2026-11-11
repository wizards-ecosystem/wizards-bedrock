# D-338 Extended-state admission uses one exact live policy and a fresh initial image

Status: Accepted, 2026-09-28. Amends D-82, D-115, D-228, D-270 and D-333; closes PW-132.

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

**Rationale.** PW-132 stayed open until the policy, image and admission edges
were each made causally red. On 2026-09-28 mutations 431, 432, 717, 310,
1,170, 828 and 829 each failed its exact check, and D-333 records the
final-door mutation 1,171. VMware's compacted `XSAVEC` execution remains
PW-305, a separate hardware-form qualification, not missing local evidence
for these invariants.

**Alternatives.** Treating any nonempty permitted subset as sufficient was
rejected because code may use every fixed component. Allowing extra live XCR0
bits was rejected because the allocated image does not promise their layout.
Saving a task once merely to initialize it was rejected because architectural
zeros already encode the defined initial state. Remembered enablement was
rejected at either user door because only current control-register state can
authorize the crossing.
