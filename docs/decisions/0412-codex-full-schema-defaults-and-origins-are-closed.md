# D-412 Codex full-schema defaults and origins are closed

Status: Accepted, 2026-10-15. Amends D-411 and D-406.

**Decision.** The reader removes only fields whose values it independently
parses and constrains, then binds the canonical remainder to the SHA-256 digest
of the qualified Codex binary's reviewed defaults. Ambient configuration must
have no origins. Effective configuration may have only origins that the owned
safety overrides can create; every required safety origin remains mandatory.
The Codex executable and runtime tree remain byte-pinned, so an originless
default is admitted only with that exact binary and complete canonical default
projection. Mutations 2,066 and 2,067 respectively accept arbitrary default
projections and self-authorize every origin; both fail
`effective_config_schemas_and_safety_fields_fail_closed`.

**Rationale.** The app-server began returning its complete effective schema,
whose fields outside the policy projection have no origin. Ignoring newly
emitted fields would let a future default activate an unreviewed side effect,
and overriding every default from the CLI would duplicate a large, unstable
runtime schema and could change Codex semantics. Exact canonical binding keeps
the broad schema closed while leaving the small owned policy explicit.
