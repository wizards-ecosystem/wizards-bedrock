# D-285 Refused device retirement remains a reachable linear owner

Status: Accepted, 2026-09-05. Amends D-199, D-245, D-251 and D-278; advances PW-533 and PW-536.

**Context.** Several bounded cleanup paths preserved an exact owner on their
first refusal but discarded it, panicked or returned an ownerless boolean on a
second refusal. This affected PCI function/device leases, BAR mappings,
generic device mappings and I/O-APIC routing. NVMe stop also treated
`CSTS.RDY = 0` alone as sufficient after clearing `CC.EN`, without freshly
proving that both registers described a disabled controller.

**Decision.** A recyclable NVMe stop token is minted only from fresh
`CC.EN = 0` and `CSTS.RDY = 0` observations of the same mapped controller, and
retains that typed observation until DMA state is retired. Every other stop or
cleanup outcome carries or stores the exact remaining owner. Bounded static
quarantines retain controller and PCI device together, BAR mapping or release
authority, device and bare-function leases, and I/O-APIC mapping or routing
authority. Selection and interrupt routing stop cleanly after quarantine;
they do not continue from an ownerless success or panic merely because safe
retention was required.

**Alternatives and proof obligation.** Infinite retry was rejected because a
device or ignored hardware write can remain unavailable forever. `forget` and
terminal panic were rejected as ownership protocols because neither leaves a
typed, inspectable recovery route. Fourteen structural DMA/cleanup contracts
cover the mint and every production sink. Ten individual manual breaks of
the CC read, CSTS read, classifier, stop classification, controller store,
memory-quarantine classification, repeated PCI release, repeated device
unmap, routing retention and panic-before-retention each made its exact witness
red before restoration.
The retained mutations and current machine boots remain PW-533/PW-536's close.
