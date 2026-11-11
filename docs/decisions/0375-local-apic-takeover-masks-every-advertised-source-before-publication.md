# D-375 Local APIC takeover masks every advertised source before publication

Status: Accepted, 2026-10-10. Amends D-33 and D-336; closes PW-584.

**Decision.** `x86-apic` decodes the independent maximum-LVT-entry field and
names entries zero through six in architectural index order: timer, thermal,
performance, LINT0, LINT1, error and CMCI. Takeover requires at least entry
five because this kernel owns APIC error delivery, accepts entry six when
advertised, and refuses a higher maximum because it would describe a source
the kernel cannot yet silence. Under the initialization transaction's
interrupt exclusion, every advertised entry is written masked and its mask bit
is read back immediately. Any missing, unknown or unmaskable source returns
`NotResponding`; dropping the transaction terminally poisons the unpublished
controller. Only after this sweep may the kernel reconfigure its error source,
software-enable the controller and publish readiness.

The readback tests the mask bit rather than requiring every returned bit to
equal the written reset value. Delivery-status and other read-only fields need
not echo a write, while the mask is the exact property that prevents retained
firmware delivery.

**Rationale.** Codex Security occurrence `occ_0569e6f2518e11797af06c04`
found that local-APIC initialization promised to mask firmware-programmable
sources but named only the timer and LINT0/LINT1. The thermal-monitor and
performance-counter LVT entries, and CMCI when implemented, could retain an
armed firmware vector because `Version` discarded the advertised LVT extent,
so a later thermal, PMU or corrected machine-check event could reach the fatal
unknown-vector path or collide with an unrelated privileged handler after
publication.

**Alternatives.** Unconditionally touching CMCI would write an unimplemented
MSR on six-entry parts. Assuming firmware reset values would preserve the
original cross-owner trust. Accepting a future maximum and masking only known
entries would recreate the defect at the next architectural extension. A
platform allowlist would move an architectural register fact into fragile
machine identity policy. Version-bounded enumeration is both narrower and
fail-closed.

**Consequences.** Host tests distinguish the version byte from the
maximum-LVT field and anchor every new register to the manual's xAPIC offset
and x2APIC MSR in architectural index order. A production-path machine fixture
proves that a six-entry APIC touches no CMCI register and a seven-entry APIC
masks CMCI too. For each of the seven known entries, an ignored mask write or
a cleared readback poisons initialization, as does an inventory ending before
error or after CMCI. Mutations 1,760 through 1,765 each fail their exact
version-field, register-order or APIC takeover witness.
