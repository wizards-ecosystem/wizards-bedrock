# D-409 Codex login status may use stderr

Status: Accepted, 2026-10-15. Amends D-406.

**Context.** After D-408 admitted the ordinary Windows Codex home, the owned
Deep qualifier reached its login probe but rejected an authenticated ChatGPT
session. The qualified Codex runtime exited successfully and printed
`Logged in using ChatGPT` on stderr, after non-fatal runtime warnings, while the
probe inspected stdout alone. No access advisory or Deep scan had started.

**Decision.** The login probe decodes both bounded output streams, extracts
trimmed `Logged in using ...` status lines from either one and admits exactly
one `Logged in using ChatGPT` line. Missing, non-ChatGPT or conflicting status
lines remain refused. Executable identity, successful exit, credential hard-link
identity and the surrounding hermetic policy remain independently enforced.

**Proof.** The focused host test
`chatgpt_login_probe_accepts_stderr_status_but_rejects_other_or_ambiguous_statuses`
accepts the current stderr form with an unrelated warning, rejects API-key
authentication and rejects conflicting stream results. Mutation 2,063 removes
stderr from the status parser and fails that witness.

**Alternatives.** Requiring a fresh user login was rejected because the live
credential is valid and the probe was reading the wrong stream. Accepting any
successful `codex login status` exit was rejected because qualification must
prove the required ChatGPT authentication channel rather than infer it.
