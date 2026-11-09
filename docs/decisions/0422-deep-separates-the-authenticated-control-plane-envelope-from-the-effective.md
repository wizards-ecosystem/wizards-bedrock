# D-422 Deep separates the authenticated control-plane envelope from the effective default-deny worker profile

Status: Accepted (2026-10-18), implemented with qualification OPEN; amends D-406, D-416, D-417, D-419 and D-420; advances PW-026

**Context.** The complete 2,133-entry mutation sweep on `0dfcb76` executed to
completion and caught 2,132 entries, with no inconclusive result. Permanent
mutation 2,036 survived because the owned-metadata witness parsed the parent
permission profile but did not independently assert its required `managed`
type. Investigation of that missing assertion reproduced a separate provider
boundary defect before another owned Deep launch: official pinned Codex
Security 0.1.32 rejects a special root deny in the supplied parent metadata,
requires a root-readable parent envelope, generates a root-readable named
profile, and compares the app-server's observed profile byte-for-byte with that
generated value. D-420's wrapper instead rewrote that profile for both
app-server and exec. The real workflow would therefore stop at provider
preflight before Daybreak or worker execution even though the direct MXC
sentinels pass. This is a fail-closed integration defect rather than an
attacker escape, but it invalidates the claimed qualification route and blocks
release.

The installed package is proprietary and supplies no redistribution or
modification grant, while no newer compatible official package is available.
Vendoring or byte-patching it is therefore not a Bedrock release option.
Putting the whole authenticated controller inside the worker sandbox is also
rejected: the controller needs the projected login, network and private state,
and nested Windows broker behavior does not prove that a later sandbox token
intersects with an outer token or Job.

**Decision.** The authenticated app-server is a measured trusted control plane,
not a model worker. Metadata supplies the provider's least permissive accepted
envelope: one `managed` profile with root read, the complete ordered set of
absolute explicit denies, restricted network, the owned source working
directory and MXC selected. The wrapper validates the provider-generated named
permission profile exactly and preserves it unchanged while injecting only the
fixed and dynamic safety overrides and final MXC selection, so the provider's
own preflight remains authoritative. The app-server may perform configuration,
account and API orchestration only; it is not authority for repository-selected
commands or model-selected tools.

Every `codex exec`, initial or resumed, remains a required wrapper crossing.
Only after validating the complete provider invocation, source-bound private
context and exact provider profile does the wrapper replace that profile
exactly once with a dynamically rendered effective profile. That effective
profile denies root by default, admits `:minimal`, grants read only to the
owned repository, scan and plugin trees and identity-covered runtime paths,
retains every nested repository, Python and credential deny, disables network,
forces MXC safety-last, disables the daemon and ignores ambient user
configuration and rules. The measured real Codex executable receives no
wrapper-control environment. Missing, duplicate, reordered, broadened or
unrepresentable policy input fails closed.

As amended by D-424 and the identity-bearing v3 transition proof, the owned
environment is `xtask-owned-codex-security-hermetic-v9` and execution authority
is `xtask-mcp-wrapper-windows-job-v8`. The sealed worker-permission description
must distinguish the provider envelope from the effective model-worker
authority rather than claim that one unchanged profile governs both layers.
The request and qualification record bind canonical SHA-256 digests of the
provider metadata state, provider named profile, effective exec profile and
effective MXC sandbox state in addition to their semantic tuple; the owned
request advances to v3 for its required request fields. A digest disagreement,
wrapper bypass or missing observed transition prevents PASS.

**Proof.** Focused witnesses independently assert the metadata profile type,
the provider-compatible root-read envelope and its exact denies, app-server
pass-through, initial and resumed exec-only replacement, strict root deny,
minimal and every positive grant, every explicit deny, network denial,
MXC-last ordering, environment removal and profile digests. Protocol fixtures
construct their expected metadata independently of the producer. Permanent
mutation 2,036 remains active and must be caught; obsolete both-command rewrite
mutations are retired or retargeted, and new permanent mutations remove or
broaden one side of the control-plane/worker boundary.

Runtime proof is deliberately split across two non-substitutable observations.
The real plugin route may launch multiple app-server children across parallel
or retried worker attempts; each real child produces a launch receipt only
after spawn, with the provider-generated permission profile preserved
unchanged. Every initial or resumed exec relays JSONL transparently but seals
its actual `thread.started` identity before forwarding that event. A zero-exit
receipt is valid only when paired to the exact same process, worker, arguments,
profile and thread start. The ledger retains the completion's exact `threadIds`
and `executionThreadIds`; the nonempty execution set must be a subset of the
completed thread set and every identity in it must have an observed initial
start, while every resumed start must have same-worker initial ancestry. A
failed, killed, mismatched or unobservable transition cannot satisfy those
relations. Before MCP, pinned Codex/MXC explicitly compiles and selects the
exact effective
named profile used by attenuated exec, includes managed requirements, forces
MXC safety-last and resolves it from the owned non-repository scan root. It
exercises every planned allowed and denied read, exactly two denied fresh-file
creates (one in the readable evidence area and one outside the allowlist),
denied append, truncate and overwrite attempts against an existing readable
canary, a denied loopback listener, a denied outbound loopback connection and a
descendant's denied read. The canonical worker-transition ledger joins those
identity-bearing transitions and the preflight, binds the provider-profile,
effective-profile and
independently rendered effective-state digests, and records seven probe counts
exactly matching the executed plan: allowed reads, denied reads, denied write
creates, denied existing-file mutations, denied network listeners, denied
network connections and denied descendants. The fixed artifact MCP is admitted
only as its existing measured, argument-constrained broker; the worker must not
be able to forge qualification evidence directly. The retained thread
identities validate every provider-reported execution thread's exact
default-profile wiring, while the explicit preflight exercises the pinned
runtime's named-profile resolver and enforcement. This combined proof does not
claim that the proprietary model selected each negative operation or that both
CLI paths have identical argument shapes. A complete real Deep run must succeed
without fallback, bind the ledger and remeasure source, plugin, runtimes,
profiles and artifacts before PASS.

**Qualification boundary.** The failed mutation sweep and all earlier Deep
diagnostics remain non-authoritative. This correction changes source,
qualification policy, evidence bindings and the mutation registry. PW-026 and
release remain open until one new clean frozen commit passes ordinary CI, the
entire updated mutation registry, and the real owned D-406/D-416/D-417/D-419/
D-420/D-422/D-424 workflow with the mandatory runtime boundary probes.

**Alternatives.** Retaining D-420's both-command rewrite was rejected because
the provider detects the app-server mismatch before scanning. Sending the
strict parent metadata unchanged was rejected because the pinned provider
rejects it. Keeping root read for model workers was rejected because explicit
secret denies cannot enumerate unrelated host data. Patching the proprietary
plugin was rejected without an upstream licensed release. Sandboxing the whole
controller was rejected because it either removes capabilities required for
authentication and API use or silently relies on unproved nested-sandbox
intersection. Configuration-only tests were rejected as release authority.
Conversely, requiring the proprietary model to select each negative operation
was rejected as nondeterministic: identity-bearing real-plugin transitions and
the exact named-profile Codex/MXC enforcement preflight are both mandatory.
