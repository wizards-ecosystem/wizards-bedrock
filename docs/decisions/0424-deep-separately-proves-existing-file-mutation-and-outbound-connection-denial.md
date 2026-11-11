# D-424 Deep separately proves existing-file mutation and outbound connection denial

Status: Accepted, 2026-10-18. Implemented with qualification OPEN; amends D-422 and D-423; advances PW-026.

**Decision.** The exact effective named profile remains the authority under
test. Before its negative probes, the owner runs bounded positive controls on
disposable private state so creation, append, truncate, overwrite, loopback
listen and loopback connect are known to work on the host. The sandboxed
preflight then retains its two denied fresh-file creates and separately attempts
append, truncate and overwrite against the existing readable worker canary.
Before and after each attempt it requires that canary to remain the same real
file with its exact bytes unchanged. Network proof likewise keeps the denied
loopback listener and adds a denied outbound connection to a parent-owned IPv4
loopback listener that remains live for the probe; no public network or DNS is
involved. Only the silent permission-denied probe status is success. A
successful operation, another error, changed canary, missing positive control,
non-loopback endpoint, timeout or unobserved child fails closed.

The owned environment advances to
`xtask-owned-codex-security-hermetic-v9`, execution authority advances to
`xtask-mcp-wrapper-windows-job-v8`, and the canonical worker-transition ledger
advances to `wizard-codex-security-worker-transitions-v3`. Its preflight tuple
has seven independent, plan-exact counts: allowed reads, denied reads, denied
fresh-file creates, denied existing-file mutations, denied network listeners,
denied network connections and denied descendants. The required operation
cardinality is two creates, three existing-file mutations, one listener, one
connection and one descendant. An aggregate write or network count cannot
substitute for either split observation.

The v3 transition half is identity-bearing rather than count-only. Each real
app-server child creates a launch receipt for its worker attempt after spawn;
multiple launches are permitted. Each initial or resumed exec must emit an
actual `thread.started` event, and its receipt is sealed before that event is
forwarded to the provider. A zero-exit receipt must pair with the same process,
worker, arguments, profile and thread start. The ledger retains both exact
completion sets, requires nonempty `executionThreadIds` to be a subset of
`threadIds` and covered by observed initial starts, and requires every resumed
start to have same-worker initial ancestry. Extra initial starts from abandoned
fresh retries may remain in the ledger; they cannot substitute for a reported
execution identity.

Focused host witnesses exercise every positive control, every private probe
dispatch, strict loopback endpoint validation, exact-profile argument
construction, unchanged-canary verification, v3 ledger render/parse/replay,
exact completion-set retention, provider-ID coverage, same-worker resume
ancestry, start/exit pairing and all four split record fields. Permanent
mutations remove or fabricate each new operation, relax each exact count or
restore the aggregate schema and red their named witness, and a fresh
immutable post-patch scan must find none of the reported gaps. These are
implementation and correction checks only; they do not mint the owned Deep
PASS.

**Qualification boundary.** PW-026 and release remain open. One new clean
frozen commit must pass ordinary CI, the complete updated mutation registry and
the real owned D-406/D-416/D-417/D-419/D-420/D-422/D-424 workflow. That run must
compile and select the exact effective named profile, execute the full split
probe plan, bind its v3 ledger to the six canonical Deep artifacts and replay a
complete zero-finding result into final evidence.

**Rationale.** Immutable-diff scan `07b1e0b0-b33c-4e0b-9f63-00fbb1619bcf`
against snapshot
`codex-security-snapshot/v1:sha256:6ebf6eb26fc11ac402368674e6a4b359820668f3a5137f2ef2b29cbadb97bc72`
reported two Low-severity, Medium-confidence findings,
`sandbox.existing-file-write-probe` and `sandbox.network-egress-probe`: both
write probes used `create_new(true)` on absent paths, and binding
`127.0.0.1:0` proves listener denial, not outbound connection denial. Neither
reproduced an escape, but the qualifier claims write and network denial, so
both gaps blocked release. Scan `48614dd5-45eb-4833-b549-67809a44cceb` found
that anonymous transition counts did not prove every reported execution thread
crossed the attenuating wrapper, and that a singular post-exit app-server
receipt could not describe a provider that stops a wrapper after launch or runs
parallel and retried workers. Listening and connecting exercise different
authority, existing-file mutation denial does not prove fresh-name denial,
append, truncation at open and overwrite reach distinct host operations, an
Internet endpoint would add external availability and disclosure to a local
proof, and aggregate counts could let a fabricated result in one class mask an
omitted probe in another.
