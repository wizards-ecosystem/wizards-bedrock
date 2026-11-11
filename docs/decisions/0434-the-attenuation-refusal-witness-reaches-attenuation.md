# D-434 The attenuation-refusal witness reaches attenuation

Status: Accepted, 2026-10-23. Qualification OPEN; strengthens D-248; advances PW-715.

**Decision.** The reservation-cleanup machine fixture owns a third identity
with `DUPLICATE | INSPECT` and asks for the meaningful additional `WAIT` right.
Before posting, it resolves the actual third request and proves that
duplication is permitted, WAIT is absent from the grant, the requested rights
are meaningful for that object, and attenuation refuses them. All three
refusals must return zero values and leave three live handles with no promised
slot. Retained mutations independently restore the earlier-gate request and
leak the reservation only from attenuation's refusal branch; both exact machine
reds and byte-restored passes are required before closing PW-715.

**Rationale.** The third request had used a handle without `DUPLICATE`, so it
failed an earlier permission gate and never tested cleanup when attenuation
itself refused. Changing only the expected status would keep that blind spot,
and testing a separate source handle would not bind the submitted request.
