# D-434 The attenuation-refusal witness reaches attenuation

Status: Accepted, 2026-10-23. Qualification OPEN; strengthens D-248; advances PW-715.

The third request in the reservation-cleanup machine fixture used a handle
without `DUPLICATE`. It failed an earlier permission gate, so the test did not
establish cleanup when attenuation itself refused the request.

The fixture now owns a third identity with `DUPLICATE | INSPECT` and asks for
the meaningful additional `WAIT` right. Before posting, it resolves the actual
third request and proves that duplication is permitted, WAIT is absent from
the grant, the requested rights are meaningful for that object, and attenuation
refuses them. All three refusals must return zero values and leave three live
handles with no promised slot.

Changing only the expected status would preserve the blind spot. Testing a
separate source handle would not bind the submitted request. The retained
mutations therefore independently restore the earlier-gate request and leak
the reservation only from attenuation's refusal branch. Both exact machine
reds and byte-restored passes are required before closing PW-715.
