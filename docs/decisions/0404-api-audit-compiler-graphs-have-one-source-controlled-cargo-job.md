# D-404 API-audit compiler graphs have one source-controlled Cargo job

Status: Accepted, 2026-10-13. Amends D-213 and D-403; advances PW-001.

**Decision.** Every Cargo compiler graph owned by the API audit carries exactly
one source-controlled `--jobs=1` argument. Graphs, rounds, instrumentation,
structured diagnostics, compiler identity, build-script authority and source
freshness checks remain unchanged; only concurrent compiler scheduling inside
one graph is serialized. A host contract requires the exact owned argument,
and permanent mutation 1,869 removes it.

**Rationale.** The sweep on `6045d10` reached API-audit round 12 before the
`xtask` test-harness compiler failed an allocation while Cargo compiled six
crates at once, a peak-concurrency failure distinct from D-403's resident
reads. The release tool must bound its own compiler concurrency rather than
retry under incidental memory; ambient `CARGO_BUILD_JOBS` is removed by the
audit and would leave the bound outside source authority; reducing graph, round
or diagnostic coverage would weaken the public-API proof; and stopping
unrelated services is outside repository authority.
