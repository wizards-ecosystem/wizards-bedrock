# D-420 Deep workers use an owned default-deny source clone

Status: Accepted, 2026-10-17. Implemented with qualification OPEN; amends D-406, D-417 and D-419; advances PW-026.

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

Replacement mutations 2,033 through 2,035 and new mutations 2,125 through 2,150
each remove or broaden one of these controls and name an exact red witness;
retained mutations continue to cover MXC selection, credential identity and
link cardinality, effective configuration, diagnostic secrecy, final override
order, sealed policy and file-URL serialization. Sealed post-patch scan
`24ea21fd-3abf-495b-b5f9-a1c0e99e8ddf` reported no findings against snapshot
`codex-security-snapshot/v1:sha256:1d2d590e07e6980622dff040021f3b75ff5b77ac1cc3b727b62c0b0d58d62a40`;
it and the non-secret runtime matrices are correction evidence, not release
qualification.

**Qualification boundary.** D-420 changes the source after every result on
`4d4ab8c`. PW-026 remains open until one new clean frozen commit passes the
complete registry and the real owned D-406/D-416/D-417/D-419/D-420 workflow,
then replays its canonical zero-finding artifacts into final evidence.

**Rationale.** The owned attempt on `4d4ab8c` stopped because its credential
sentinel could open the projected `auth.json` while the log reported `MXC
availability: true` and `Windows sandbox selection: disabled`. A non-secret
fixture showed that the unelevated selector, `useMxc` alone and MXC without a
file deny all read the file (exit 74), and only MXC plus an exact deny refused
it (exit 73); the runtime also requires `sandboxCwd` as a canonical Windows file
URL. Scan `8fc00696-4491-4db4-bec0-6df1cbceb083` (snapshot
`00b19fc9a9266a686d8e0c5945b76c6a8b6791115d9be54d4819d60fe099c2dd`)
reproduced a High-severity bypass through the live
`%USERPROFILE%\.codex\auth.json` hard-link name, and diff scan
`fa648a6b-c7a4-4f6f-933f-3ef9e3fa37e0` reported the remaining host-root read as
Medium finding `csf_fb54adbba70a8faebc6b1451`, occurrence
`occ_25625841eed533f656ff28b8`, against snapshot
`codex-security-snapshot/v1:sha256:4160aa136faa808980771acf66f72297973f870870d67033ab70114b732e42b2`.
Both scans are correction evidence, not qualification authority.

Accepting exit 74, removing the probe, relying on an ambient Windows setting or
the unelevated backend each leaves credential content readable, and fixing
only the sentinel would leave model workers on the disabled backend.
Diagnostics are not execution authority, so the effective configuration is
verified. Denying one name, or both names without constraining link count,
leaves an alias. Root read with more denies cannot enumerate every unrelated
host secret. The live checkout carries ignored artifacts, reparse points,
alternate streams and hard-link aliases outside Git's tracked-source identity.
An explicit deny on the outside canary would still pass after a regression to
root read, and broad Node, Python or Git installation grants cover unread bytes
outside the sealed runtime identities.
