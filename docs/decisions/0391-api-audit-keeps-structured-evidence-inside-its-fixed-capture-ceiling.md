# D-391 API audit keeps structured evidence inside its fixed capture ceiling

Status: Accepted, 2026-10-12. Amends D-213 and D-275.

**Context.** The semantic API audit deliberately causes one compiler warning
per instrumented public item and consumes Cargo's structured JSON diagnostic
fields. Cargo's ordinary `json` message format also embeds the full human
rendering of every diagnostic. The audit does not consume that rendering, but
the growing `xtask` test harness repeated enough source context there to cross
the existing 64 MiB combined-output ceiling during antichain round 15. Raising
the ceiling would spend more transient disk and memory on bytes which carry no
audit authority and would only postpone the same failure as the API grows.

**Decision.** Every audited Cargo compilation requests
`json-diagnostic-short`. Cargo still emits the same structured package,
target, code, level, span, expansion and artifact fields consumed by the
audit, while its unused `rendered` member is compact. The 64 MiB capture bound
and its fail-closed polling remain unchanged. Metadata and registry
materialization retain their existing bounded paths; only compiler diagnostic
rendering changes.

**Proof required.** The command-construction unit must require the compact
structured format together with offline and lockfile-frozen execution. A
permanent mutation restoring ordinary JSON must fail exactly that unit. The
complete API audit and ordinary CI gate must then pass at the current public
surface without increasing the capture limit.

**Alternatives.** Increasing the limit retains avoidable amplification and
works against constrained development machines. Parsing a human short format
would discard the structured origin and span evidence. Streaming and reducing
Cargo JSON could lower peak memory further, but it is a larger trust-boundary
change than selecting Cargo's supported compact structured representation.
