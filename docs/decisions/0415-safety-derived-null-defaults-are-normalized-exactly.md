# D-415 Safety-derived null defaults are normalized exactly

Status: Accepted, 2026-10-15. Amends D-412 and D-406.

**Decision.** After removing only the three independently constrained memories
fields, the full-schema check may normalize the twelve otherwise absent
`memories.*` defaults and `features.network_proxy` only when each is absent or
exactly null. A non-null value is refused before hashing, and any new field
remains in the canonical remainder covered by the existing qualified-runtime
digest. Origins and the effective values of all owned safety fields remain
independently checked. Mutation 2,069 treats every dependent value as null and
fails `effective_config_schemas_and_safety_fields_fail_closed`.

**Rationale.** Setting the three memories-disable overrides makes Codex project
those thirteen fields as explicit nulls; they were the only changes outside the
fields D-412 parses, so the raw remainder kept its pinned digest while the
effective remainder differed. A second whole-config digest would not
distinguish the raw and safety-derived phases or say why the projection
differs, and removing the fields without checking their values would let a
future active default escape both the exact-null contract and the digest.
