# D-415 Safety-derived null defaults are normalized exactly

Status: Accepted, 2026-10-15. Amends D-412 and D-406.

**Context.** The first owned Deep attempt after D-413 passed the raw originless
configuration digest, then failed before its access advisory when the same
qualified app-server read the owned safety overrides. Setting the three
memories-disable fields makes Codex project twelve otherwise absent
`memories.*` defaults and `features.network_proxy` as explicit nulls. A
credential-free reproduction showed that these were the only changes outside
the fields already parsed and constrained by D-412: the raw remainder retained
its pinned digest, while the unnormalized effective remainder differed.

**Decision.** After removing only the three independently constrained memories
fields, the full-schema check may normalize those thirteen runtime-dependent
fields only when each is absent or exactly null. A non-null value is refused
before hashing, and any new field remains in the canonical remainder covered by
the existing qualified-runtime digest. Origins and the effective values of all
owned safety fields remain independently checked.

**Proof.** The fake app-server now reproduces the complete null projection in
its safe response and changes `features.network_proxy` to an active Boolean in
the negative case. The focused schema test accepts the real inert shape and
rejects the active value. Mutation 2,069 treats every dependent value as null
and fails `effective_config_schemas_and_safety_fields_fail_closed`.

**Alternatives.** Admitting a second whole-config digest was rejected because
it would not distinguish the raw and safety-derived phases and would obscure
why the projection differs. Removing the dependent fields without checking
their values was rejected because a future active default would escape both
the exact-null contract and the canonical digest.
