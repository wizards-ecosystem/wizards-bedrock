# D-257 APIC readiness follows an exact software-enable readback

Status: Accepted, 2026-08-31. Amends D-33 and D-254; advances PW-126.

**Decision.** The `ConfigureSources` stage reads the SVR immediately after its
write and requires the complete value to equal the typed `Spurious` encoding.
Only that exact observation permits vector-identity stores, the identity stage
and readiness publication. A mismatch returns `NotResponding`; the transaction
restores its entry interrupt state, publishes no vector identity, terminally
poisons initialization and refuses retry.

**Rationale.** The final initialization write sets the spurious vector and the
SVR software-enable bit, but the transaction published its vector identities
and `Ready` without reading it back, and `is_enabled` checked only after
readiness, when it could no longer retract the initialization that lets boot
continue toward maskable-interrupt admission. An ignored write therefore
admitted a ready but disabled controller.

**Alternatives and evidence.** Checking only bit 8 was rejected because an
altered vector would deliver through the wrong trap entry. Rechecking in the
kernel caller was rejected because it would leave the safe `apic::init`
contract false for every other caller. Trusting `wrmsr` was rejected because
the safety-dependent value is readable and ignored writes are an explicit
hostile surface. A typed fake retains normal writes and ignores only the SVR
write; it proves the readback refuses with zero vector identities and a
poisoned, unavailable controller. Deleting only the comparison fails that
single machine check, and the exact break is retained in the mutation registry.
