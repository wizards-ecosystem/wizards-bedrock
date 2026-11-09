# D-257 APIC readiness follows an exact software-enable readback

Status: Accepted, 2026-08-31. Amends D-33 and D-254; advances PW-126.

**Context.** The final local-APIC initialization write sets the spurious vector
and the SVR bit which software-enables the controller. The transaction then
published its vector identities and eventually `Ready` without reading that
write back. `is_enabled` checked the register only after readiness; it could
make individual operations unavailable but could not retract the successful
initialization which lets boot continue toward maskable-interrupt admission.
An ignored write therefore admitted a ready but disabled controller.

**Decision.** The `ConfigureSources` stage reads the SVR immediately after its
write and requires the complete value to equal the typed `Spurious` encoding.
Only that exact observation permits vector-identity stores, the identity stage
and readiness publication. A mismatch returns `NotResponding`; the transaction
restores its entry interrupt state, publishes no vector identity, terminally
poisons initialization and refuses retry.

**Alternatives and evidence.** Checking only bit 8 was rejected because an
altered vector would deliver through the wrong trap entry. Rechecking in the
kernel caller was rejected because it would leave the safe `apic::init`
contract false for every other caller. Trusting `wrmsr` was rejected because
the safety-dependent value is readable and ignored writes are an explicit
hostile surface. A typed fake retained normal writes and ignored only the SVR
write; it then proved the readback refused with zero vector identities and a
poisoned, unavailable controller. Deleting only the comparison failed that
single machine check at 246/247. The restored release boot passed 247/247, the
82 APIC host tests and doctest passed, and the exact break is retained in the
mutation registry.
