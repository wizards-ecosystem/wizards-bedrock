# D-412 Codex full-schema defaults and origins are closed

Status: Accepted (2026-10-15), amends D-411 and D-406

**Context.** After D-411 admitted the hermetic initialization identity, the
qualified Codex app-server returned its complete effective-configuration schema
instead of the earlier compact projection. The private home was still the exact
empty configuration, and every returned field outside the qualifier's policy
projection had no origin. No access advisory or Deep scan had started.

**Decision.** The reader removes only fields whose values it independently
parses and constrains, then binds the canonical remainder to the SHA-256 digest
of the qualified Codex binary's reviewed defaults. Ambient configuration must
have no origins. Effective configuration may have only origins that the owned
safety overrides can create; every required safety origin remains mandatory.
The Codex executable and runtime tree remain byte-pinned, so an originless
default is admitted only with that exact binary and complete canonical default
projection.

**Proof.** The fake app-server suite rejects both changed originless defaults
and a foreign effective origin. Mutations 2,066 and 2,067 respectively accept
arbitrary default projections and self-authorize every origin; both fail
`effective_config_schemas_and_safety_fields_fail_closed`.

**Alternatives.** Ignoring newly emitted fields was rejected because a future
default could activate an unreviewed side effect. Treating every default field
as a CLI override was rejected because it would duplicate a large, unstable
runtime schema and could change Codex semantics. Exact canonical binding keeps
the broad schema closed while leaving the small owned policy explicit.
