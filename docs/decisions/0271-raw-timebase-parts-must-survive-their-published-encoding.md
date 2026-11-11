# D-271 Raw timebase parts must survive their published encoding

Status: Accepted, 2026-08-31. Amends D-166; advances PW-309.

**Decision.** Raw construction is checked and returns `None` unless the
multiplier fits its 32-bit published field and the shift is at most 63. Zero
remains the valid uncalibrated conversion. Every admitted pair must round-trip
exactly through `to_bits` and `from_bits`; decoding arbitrary bits remains total
by masking into those same bounds.

**Rationale.** `Timebase::from_parts` was public but admitted shifts which cannot
be evaluated and multipliers wider than the one-word time-page representation.
Debug could panic on the shift while release silently decoded different fields.

**Alternatives and current evidence.** Making `from_parts` private was rejected
because host and ABI code legitimately reconstruct checked published fields.
Masking at construction was rejected because it would accept one value and
silently store another. Boundary and complete-shift round-trip host tests cover
the decision, and deleting either bound fails its distinct retained mutation.
Debug, release and current page consumers remain the register's final proof.
