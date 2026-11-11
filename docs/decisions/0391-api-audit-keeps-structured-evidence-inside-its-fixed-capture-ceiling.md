# D-391 API audit keeps structured evidence inside its fixed capture ceiling

Status: Accepted, 2026-10-12. Amends D-213 and D-275.

**Decision.** Every audited Cargo compilation requests
`json-diagnostic-short`. Cargo still emits the same structured package,
target, code, level, span, expansion and artifact fields consumed by the
audit, while its unused `rendered` member is compact. The 64 MiB capture bound
and its fail-closed polling remain unchanged. Metadata and registry
materialization retain their existing bounded paths; only compiler diagnostic
rendering changes. The command-construction unit requires the compact
structured format together with offline and lockfile-frozen execution, and a
permanent mutation restoring ordinary JSON fails exactly that unit.

**Rationale.** Cargo's ordinary `json` format embeds the full human rendering
of every diagnostic, which the audit does not consume, and the growing `xtask`
test harness repeated enough source context there to cross the 64 MiB ceiling.
Raising the ceiling would spend transient disk and memory on bytes with no
audit authority and only postpone the failure; parsing a human short format
would discard the structured origin and span evidence.
