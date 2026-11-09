# D-420 Deep workers use an owned default-deny source clone

Status: Accepted (2026-10-17), implemented with qualification OPEN; amends D-406, D-417 and D-419; advances PW-026

**Context.** The owned Deep attempt on `4d4ab8c` passed the ordinary CI and
native-debug gates, then stopped before Daybreak, MCP worker launch or scan
creation because its live credential sentinel could open the projected
`auth.json`. Failure cleanup unlinked that exact private credential and minted
no qualification authority. The retained non-secret sandbox log reported both
`MXC availability: true` and `Windows sandbox selection: disabled`. A separate
harmless fixture containing no login secret reproduced the boundary: an
unelevated selector, `useMxc` alone and MXC without a file deny each returned
the fixture's readable exit 74; only MXC plus an exact deny for the projected
auth path returned silent permission-denied exit 73. The probe also established
that this runtime requires `sandboxCwd` as a canonical Windows file URL rather
than a native path string. An independent immutable-diff review then reproduced
a High-severity alias bypass against a non-secret hard-link fixture: denying
only the projected name still allowed the same file to be opened through its
live `%USERPROFILE%\.codex\auth.json` name. Scan
`8fc00696-4491-4db4-bec0-6df1cbceb083` binds that finding to snapshot
`00b19fc9a9266a686d8e0c5945b76c6a8b6791115d9be54d4819d60fe099c2dd`.
The resulting dual-name fix still left the managed worker with host-root read;
repository-controlled instructions could therefore steer it toward unrelated
host secrets. Formal diff scan `fa648a6b-c7a4-4f6f-933f-3ef9e3fa37e0`
reported that Medium finding as `csf_fb54adbba70a8faebc6b1451`, occurrence
`occ_25625841eed533f656ff28b8`, against snapshot
`codex-security-snapshot/v1:sha256:4160aa136faa808980771acf66f72297973f870870d67033ab70114b732e42b2`.
Both scans are correction evidence, not qualification authority.

**Decision.** The owned environment advances to
`xtask-owned-codex-security-hermetic-v6`, execution authority advances to
`xtask-mcp-wrapper-windows-job-v5`, and the existing sealed worker-permission
field now binds exactly
`backend=mxc;root=deny;minimal+owned-repository+scan+plugin+qualified-runtime=read;repository-metadata+credential=deny;outside-canary=default-deny-probed;network=false`.
`windows.sandbox="mxc"` is the unique final configuration override after all
provider, ambient-route neutralization and worker-artifact settings. The
effective-config reader requires both the exact `mxc` value and its explicit
origin. The wrapper accepts the provider's one exact root-readable named
profile only to rewrite it exactly once to root-deny before process launch; no
effective app-server or worker invocation retains root read.

The live credential must begin with exactly one hard-link name; the owned
projection must make exactly two names for one file identity, and any
pre-existing or later third alias fails closed. Workers never scan the live
checkout. `xtask` creates a local `--no-checkout --no-hardlinks --local
--no-tags` clone below its private scratch, writes every clean tracked source
file into it as new bytes, resets only the index to the frozen commit, removes
`origin`, and proves the complete source identity before and after execution.
This excludes ignored build products and live-checkout aliases from the worker
input while retaining the exact `git_revision` target identity.

One shared managed state denies special root by default, admits `:minimal`,
and grants read only to the owned clone, plugin mirror, scan root, exact Node
executable, qualified Python root files plus its measured `DLLs`/`Lib` trees,
and the exact four-file Codex runtime. It explicitly denies clone `.git`, every
unqualified Python `site-packages`/`__pycache__` subtree and both credential
names, denies network access, sets `useMxc: true`, and serializes the owned
clone as a canonical file-URL working directory. Before launch, pinned-runtime
sentinels prove every allowed category readable and prove silent denial for
`.git`, both credential names and a harmless canary directly beneath the
otherwise unreadable scratch root. That canary deliberately has no explicit
deny, so its exit 73 establishes the root default itself. Every decision must
be silent and exact: allow is exit 74, deny is exit 73, and any other status or
output fails closed. The canaries, runtime, credentials and owned source are
remeasured around and after their last use, and Daybreak, start and completion
receive the same qualified policy object.

**Proof.** The focused configuration, wrapper, protocol, request, sealing and
credential suite covers exact effective value and origin, clone construction
and rebinding, file-URL encoding, root-deny/minimal/ordered exact entries,
safety-last placement for app-server, new and resumed workers, identical policy
propagation in all three MCP calls, exact two-link lifecycle, positive and
negative probes, and content-free diagnostics. Replacement mutations 2,033
through 2,035 and new mutations 2,125 through 2,150 remove or broaden one of
those controls; each names its exact red witness. Retained mutations continue
to cover MXC selection, credential identity and link cardinality, effective
configuration, diagnostic secrecy, final override order, sealed policy and
file-URL serialization. The focused Deep suite executes 59 passing tests with
two child fixtures intentionally ignored. Independent sealed post-patch scan
`24ea21fd-3abf-495b-b5f9-a1c0e99e8ddf` reported no findings against snapshot
`codex-security-snapshot/v1:sha256:1d2d590e07e6980622dff040021f3b75ff5b77ac1cc3b727b62c0b0d58d62a40`.
That scan and the non-secret runtime matrices bind their own snapshots and
remain diagnosis and correction evidence, not release qualification; the
final frozen source must still pass the owned workflow.

**Qualification boundary.** D-420 changes the source after every result on
`4d4ab8c`. PW-026 remains open until one new clean frozen commit passes the
complete registry and the real owned D-406/D-416/D-417/D-419/D-420 workflow,
then replays its canonical zero-finding artifacts into final evidence.

**Alternatives.** Accepting exit 74, removing the credential probe, relying on
an ambient Windows setting or selecting the unelevated backend was rejected
because each leaves credential content readable. Fixing only the sentinel was
rejected because actual model workers would retain the disabled backend.
Trusting `useMxc` without the explicit deny was rejected by the live matrix,
and recording log text without verifying effective configuration was rejected
because diagnostics are not execution authority. Denying only the projected
name was rejected by the reproduced hard-link bypass. Merely adding the live
name without constraining link count was also rejected because a pre-existing
third name would retain the same access path.
Keeping root read while adding more secret-path denies was rejected because it
cannot enumerate every unrelated host secret. Granting the live checkout was
rejected because ignored artifacts, reparse points, alternate streams and
hard-link aliases are outside Git's tracked-source identity. Explicitly
denying the outside canary was rejected because it would still pass after a
regression from root deny to root read. Broad Node, Python or Git installation
grants were rejected because their unread bytes are not covered by the sealed
runtime identities.
