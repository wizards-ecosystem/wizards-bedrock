# D-416 The owned Deep dialogue gives Daybreak one advisory read before scan start

Status: Accepted (2026-10-16), implemented with qualification OPEN; amends D-406; advances PW-026

**Context.** D-406 owns the full MCP process and scan dialogue, but its pinned
sequence went directly from tool discovery to Deep start. Codex Security's
Daybreak access tool reports whether the authenticated ChatGPT account can
display protected scan results. That is useful operator information, but it is
not Amazon Bedrock model entitlement, provider selection, scan authorization or
release evidence. Treating it as any of those would conflate access advice with
the authority D-406 derives only from the completed owned scan and its replayed
artifacts.

**Decision.** After MCP initialization and exact tool discovery, the same
bounded `LineDialogue` performs exactly one
`get_codex_security_daybreak_access` call before the one Deep start. Its
`arguments` object is empty; the call carries only D-406's already-owned source
and managed-sandbox metadata and cannot inject an entitlement. The discovered
descriptor is required exactly once and is byte-bound at 718 bytes with SHA-256
`44787ff68bf20a00a06e128e33a1d4ba42b0a63a5c2b61ee83dfdaddb019f09a`.

A fresh `granted` result may name only Daybreak Blue and Daybreak Red and is
displayed as information. A fresh `not_granted` result emits a prominent
warning and only a syntactically safe HTTPS enrollment URL, otherwise the fixed
`https://chatgpt.com/cyber` fallback. An `unknown` or stale result, a valid MCP
tool error, or a malformed advisory payload remains silent and the scan
continues. The advisory status never enters the PASS predicate. By contrast,
malformed JSON-RPC framing or correlation remains a fatal protocol error:
an error object needs an integer code, a non-empty control-free message and at
most optional data. This preserves D-406's transport integrity without
promoting the optional advisory payload into release authority.

**Proof required.** Protocol fixtures must require the five-step sequence
initialize, tools/list, Daybreak advisory, Deep start and completion; reject a
missing, duplicate or drifted advisory descriptor; bind the empty arguments and
owned metadata; cover granted, not-granted, unknown, stale, malformed-payload
and valid-tool-error continuations; and reject malformed error envelopes. The
wire-shape and operator-notice tests must prove that unsafe enrollment URLs use
the fixed fallback and that advisory content cannot alter scan inputs or PASS.
Mutations 2,070, 2,071, 2,078 through 2,082 and 2,084 retain the omitted advisory,
malformed URL, Unicode-control, presentation-channel and JSON-RPC-error defects
against their exact host witnesses.

**Qualification boundary.** The sequence and verifier are implemented. PW-026
remains open until a clean frozen source completes the real owned Deep run,
produces the canonical zero-finding result, passes the complete replacement
mutation sweep and is replayed into release evidence. A Daybreak Blue grant, by
itself, closes none of those gates.

**Alternatives.** Skipping the advisory discards account-specific display
guidance required by the owned workflow. Making `not_granted`, `unknown` or an
advisory-tool error stop the scan would turn non-authoritative account advice
into scan authority. Supplying account or program arguments was rejected
because the tool already observes the authenticated account and the caller has
no authority to manufacture its result.
