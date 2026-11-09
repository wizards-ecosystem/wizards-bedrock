# D-423 The active Codex tuple and normalized default projection are remeasured together

Status: Accepted, 2026-10-18. Implemented with qualification OPEN; amends D-417 and D-422; advances PW-026.

**Context.** Before D-422's corrected worker boundary could be frozen, the
ChatGPT Codex installation replaced D-417's content-addressed
`e78bad9887fa50cf` child with `896a0166b774037a`. The old directory is absent.
The entrypoint still reports `codex-cli 0.163.0-alpha.10.2`, and all four file
sizes, the aggregate byte count and the generated system-skill tree are
unchanged, but every runtime file digest and the folded tree digest changed.
Using D-417's missing path or silently selecting an ambient executable would
therefore stop or weaken the owned workflow.

Two new empty private-home probes also agreed on a canonical 3,041-byte
originless default projection with SHA-256
`d78831b7ffc95fa8e14e2cf26770c3d8138156b7496a699a3567a465929e8aea`
instead of D-417's 3,056-byte projection. The exact field-level delta is only
the removal of top-level `windows: null`: reinserting that member reproduces
the old bytes and SHA-256 exactly. This is a normalization change for the
qualifier-owned `windows.sandbox` field, not an activated ambient default.

**Decision.** The owned Codex child advances only to
`896a0166b774037a`. Its exact files are
`codex-code-mode-host.exe` (74,921,776 bytes, SHA-256
`3b6a9179a4da17fa5fc6948219cd61486dc3b534c683838076d8c61bd311c68e`),
`codex-command-runner.exe` (8,217,392 bytes, SHA-256
`ce80b56a2f37f808815fde33cceb97ac858ceecde296425ad3860f0f37fb4877`),
`codex-windows-sandbox-setup.exe` (17,700,144 bytes, SHA-256
`c4da812016cbfb92d9a62791f78460c59e9662faadb0a2720d3aa5d0535b80bd`)
and `codex.exe` (341,312,816 bytes, SHA-256
`f6dbf48cb70f62649c645f26c87dc9bf84cc36986202dfad8e12fb5b8ca70312`).
The exact four-file tree remains 442,152,128 bytes and advances to folded
SHA-256 `72dd0efdd65e34488bc013f88166a8f549cf1f7441d0d23ad7116bc5758de1d5`.
Manual provenance review also found valid Authenticode signatures from OpenAI
OpCo, LLC with certificate thumbprint
`8C1BBD222297A3669B63F95B42710343619559BB`. That observation establishes the
measured tuple's provenance; the automated qualifier pins and remeasures the
exact paths, inventory, sizes and content hashes, but does not enforce
Authenticode signer or certificate identity.

The default projection advances to the independently repeated 3,041-byte
digest above. Normalization may remove only the qualifier-owned
`windows.sandbox` value and collapse an otherwise absent, null or empty
`windows` object; other Windows defaults remain identity-bearing. The
effective configuration still requires the safety-last
`windows.sandbox="mxc"` override and its authorized origin, while rejecting a
missing origin, a different backend or an unexpected sibling value. The
generated system-skill identity remains 71 entries, 314,963 file bytes and
SHA-256 `54c2abe5f6017b67270f4bbb993b3b1e3a808da31575a1d24174cb4b2edd99c8`.

**Proof.** Independent host remeasurement verified each file's absolute
content identity and size, and the separate manual Authenticode review verified
the signer and signing-certificate thumbprint. Both empty-home probes produced
byte-identical defaults. A
field-by-field comparison found no added member and no changed common value;
programmatic reconstruction with only `windows: null` reproduced D-417's old
projection and hash. Focused Deep and qualification suites accept the new
tuple, while their existing witnesses still reject runtime, default,
effective-config, origin and unexpected-Windows-field drift. At this decision's
implementation point the active mutation registry contained 2,203 entries with
next permanent identity 2,235; that count excluded the 31 permanently retired
identities and does not include D-424's later additions.

**Qualification boundary.** These measurements establish what the owned
workflow must execute; they are not a Deep result or release qualification.
D-422 and PW-026 remain open until the new clean frozen commit passes ordinary
CI, the complete then-current mutation registry and the real owned Deep workflow,
which must remeasure this exact tuple after its last use and replay the
canonical zero-finding artifacts into final evidence.

**Alternatives.** Restoring the missing D-417 child, accepting any directory
with the same version string, trusting signatures without content hashes or
retaining only the entrypoint digest was rejected because each loses the exact
process-tree authority. Treating the removed null member as a security default
change was rejected by the byte-for-byte reconstruction. Ignoring all Windows
defaults was rejected because only the owned sandbox field may be normalized;
every unexpected sibling remains digest-bearing and fail-closed.
