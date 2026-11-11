# D-409 Codex login status may use stderr

Status: Accepted, 2026-10-15. Amends D-406.

**Decision.** The login probe decodes both bounded output streams, extracts
trimmed `Logged in using ...` status lines from either one and admits exactly
one `Logged in using ChatGPT` line. Missing, non-ChatGPT or conflicting status
lines remain refused. Executable identity, successful exit, credential hard-link
identity and the surrounding hermetic policy remain independently enforced.
The host test
`chatgpt_login_probe_accepts_stderr_status_but_rejects_other_or_ambiguous_statuses`
accepts the stderr form beside an unrelated warning and rejects API-key and
conflicting results; mutation 2,063 removes stderr from the status parser and
fails it.

**Rationale.** The qualified runtime printed its ChatGPT status on stderr after
non-fatal warnings while the probe read stdout alone, so a valid session was
refused. Accepting any successful `codex login status` exit would infer rather
than prove the required ChatGPT authentication channel.
