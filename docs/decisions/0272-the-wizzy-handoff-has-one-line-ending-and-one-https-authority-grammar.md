# D-272 The Wizzy handoff has one line ending and one HTTPS authority grammar

Status: Accepted, 2026-08-31. Amends D-226; advances PW-520.

**Context.** D-226 required a byte-stable envelope but its parser accepted both
terminal-line forms and treated the `https://` prefix as sufficient provenance.
A hostless or malformed authority is not a reachable repository identity, and
two terminal forms are not one canonical encoding.

**Decision.** A v1 handoff contains no carriage return and ends in exactly one
LF. Repository provenance uses `https://` followed by a nonempty authority: a
DNS-style name with nonempty alphanumeric-bounded labels, IPv4, or bracketed
IPv6, with an optional nonzero decimal `u16` port without leading zeroes. User
information, empty labels, invalid brackets, malformed numeric hosts and stray
authority characters are refused. Path, query and fragment bytes have no power
to repair an invalid authority.

**Alternatives and current evidence.** Accepting a generic URI string was
rejected because the envelope promises reachable HTTPS provenance, not merely
syntax containing a scheme. Normalizing alternate bytes was rejected because
the byte identity is the original envelope. Exact negative fixtures and
supported-host tables cover the grammar; terminal-LF, hostless-authority and
over-refusal mutations are registered against separate tests. This remains
specification-only: PW-601 still stops every consumer and no Wizzy artifact is
claimed.
