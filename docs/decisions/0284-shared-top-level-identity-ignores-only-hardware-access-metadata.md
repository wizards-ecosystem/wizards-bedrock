# D-284 Shared top-level identity ignores only hardware access metadata

Status: Accepted (2026-09-05), amends D-281; advances PW-540

**Context.** D-281 compared complete higher-half PML4 words when binding and
switching a user root. Hardware may independently set `ACCESSED` in either
copy, so exact word equality could reject an otherwise identical root. At the
same time, topology reservation treated any present device-window entry as an
acceptable table, including a huge or user-accessible entry.

**Decision.** Intermediate-table identity requires the same physical address
and every identical flag except `ACCESSED`. The pure comparison lives in
`x86-paging`; no other bit is normalized. Device-window reservation rejects a
present huge or `USER` entry and creates an absent entry only as a
supervisor-only table before activation freezes the topology.

**Alternatives and proof obligation.** Ignoring all hardware-looking bits was
rejected because dirty, cache, permission, execution and reserved-bit drift
are part of the mapping contract. Continuing to compare raw words was rejected
because page walks themselves can invalidate the proof. The host test toggles
all 64 bits and accepts only `ACCESSED`; machine witnesses cover accessed-only
bind drift, both hostile present entries and the supervisor reservation.
Registered mutations retain each defect. Their restored release, debug and
VMware executions remain PW-540's close.
