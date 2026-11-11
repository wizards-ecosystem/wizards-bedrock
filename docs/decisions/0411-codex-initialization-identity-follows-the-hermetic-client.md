# D-411 Codex initialization identity follows the hermetic client

Status: Accepted, 2026-10-15. Amends D-410 and D-406.

**Decision.** The initialization check binds the outer user-agent product to
the exact `codex_security_deep_scan` client name, the qualified Codex version,
the exact inner client name and version and the `unknown` terminal class created
by the qualifier's cleared environment. Only the bounded printable Windows host
segment remains variable. The private Codex home, platform, response schema,
effective configuration and all other D-406 boundaries remain exact. Mutation
2,065 substitutes the ambient `codex_vscode` product and fails
`config_reader_emits_exact_requests_and_accepts_only_the_supported_notification`.

**Rationale.** D-410 derived its `userAgent` framing from an ambient diagnostic,
but the qualifier clears inherited environment, so the runtime truthfully
reports the hermetic client and terminal class. The qualifier controls the
environment and has one expected identity, and the ambient form does not
describe the process whose effective configuration authorizes Deep.
