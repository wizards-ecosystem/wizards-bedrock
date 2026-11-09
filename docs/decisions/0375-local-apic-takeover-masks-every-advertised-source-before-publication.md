# D-375 Local APIC takeover masks every advertised source before publication

Status: Accepted, 2026-10-10. Amends D-33 and D-336; closes PW-584.

**Context.** Codex Security occurrence `occ_0569e6f2518e11797af06c04`
found that local-APIC initialization promised to mask firmware-programmable
sources but named only the timer and LINT0/LINT1. The thermal-monitor and
performance-counter LVT entries, and CMCI when implemented, could retain an
armed firmware vector. The version register advertised the implemented LVT
extent, but `Version` discarded that field. A later thermal, PMU or corrected
machine-check event could therefore reach the fatal unknown-vector path or
collide with an unrelated privileged handler after publication.

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

**Proof required.** Host tests must independently distinguish the version byte
from the maximum-LVT field, anchor every new register to the manual's xAPIC
offset and x2APIC MSR, and preserve architectural index order. A production-path
machine fixture must prove that a six-entry APIC touches no CMCI register,
a seven-entry APIC masks CMCI too, and both cover every advertised entry before
publication. For each of all seven known entries, an ignored mask write and a
cleared readback must poison initialization. Inventories ending before error or
after CMCI must do the same. The live release and debug machines must pass all
279 checks.

**Evidence.** The `x86-apic` suite passes 87 host tests and two doctests. The
restored release machine passed 279/279. Mutations 1,760 and 1,761 failed their
exact version-field and register-order host tests. Mutations 1,762 through
1,765 respectively capped the inventory before CMCI, removed mask-readback
enforcement, admitted an inventory without error and silently capped an
unknown future source; each reddened its named APIC takeover check before
source restoration. Final integrated-gate evidence is recorded by the closing
commit.

**Alternatives.** Unconditionally touching CMCI would write an unimplemented
MSR on six-entry parts. Assuming firmware reset values would preserve the
original cross-owner trust. Accepting a future maximum and masking only known
entries would recreate the defect at the next architectural extension. A
platform allowlist would move an architectural register fact into fragile
machine identity policy. Version-bounded enumeration is both narrower and
fail-closed.
