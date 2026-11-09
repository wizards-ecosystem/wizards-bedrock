# D-404 API-audit compiler graphs have one source-controlled Cargo job

Status: Accepted (2026-10-13), amends D-213 and D-403; advances PW-001

**Context.** The fresh complete sweep on `6045d10` proved its pristine host
baseline and reached API-audit round 12, but the `xtask` test-harness compiler
failed a 3,735,568-byte allocation while Cargo was compiling six crates and
emitting 25,182 expected audit warnings. D-403 removed executable-sized
resident reads, and the same API audit passed twice outside the isolated sweep,
so this was a distinct peak-concurrency failure rather than a source or audit
verdict. The machine's unrelated committed memory remains outside repository
authority.

**Decision.** Every Cargo compiler graph owned by the API audit carries exactly
one source-controlled `--jobs=1` argument. Graphs, rounds, instrumentation,
structured diagnostics, compiler identity, build-script authority and source
freshness checks remain unchanged; only concurrent compiler scheduling inside
one graph is serialized. A host contract requires the exact owned argument,
and permanent mutation 1,869 removes it.

**Proof required.** The new host contract must fail before the scheduling bound,
pass after it and fail when mutation 1,869 is applied. The complete API audit
and ordinary gate must pass. Because the interrupted sweep reached no mutation
case, a fresh complete sweep bound to the repaired commit must finish with no
survivor or inconclusive result before release publication.

**Alternatives.** Retrying under incidental memory availability was rejected
because the release tool must bound its own compiler concurrency. Ambient
`CARGO_BUILD_JOBS` was rejected because the audit deliberately removes
compiler-semantic environment and would leave the bound outside source
authority. Reducing graph, round or diagnostic coverage was rejected because
that would weaken the public-API proof. Stopping WSL, editors or unrelated
services was rejected as outside repository authority.
