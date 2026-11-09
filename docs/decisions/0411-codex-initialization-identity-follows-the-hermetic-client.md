# D-411 Codex initialization identity follows the hermetic client

Status: Accepted (2026-10-15), amends D-410 and D-406

**Context.** D-410 derived its user-agent framing from an ambient diagnostic.
The owned qualifier clears inherited environment before starting the qualified
Codex app-server, so the same runtime truthfully identifies the requesting Deep
client as `codex_security_deep_scan` and the absent terminal class as `unknown`.
The stricter owned run rejected the ambient `codex_vscode` and `dumb` literals.
No access advisory or Deep scan had started.

**Decision.** The initialization check binds the outer user-agent product to
the exact `codex_security_deep_scan` client name, the qualified Codex version,
the exact inner client name and version and the `unknown` terminal class created
by the qualifier's cleared environment. Only the bounded printable Windows host
segment remains variable. The private Codex home, platform, response schema,
effective configuration and all other D-406 boundaries remain exact.

**Proof.** The fake app-server success transcript now emits the captured
hermetic identity. Mutation 2,065 substitutes the ambient `codex_vscode`
product and fails
`config_reader_emits_exact_requests_and_accepts_only_the_supported_notification`.

**Alternatives.** Accepting either ambient or hermetic products and terminal
classes was rejected because the qualifier controls the environment and has one
expected identity. Preserving the ambient diagnostic was rejected because it
does not describe the process whose effective configuration authorizes Deep.
