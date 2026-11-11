# D-394 Mutation witnesses follow the current causal verdict

Status: Accepted, 2026-10-12. Amends D-239 and D-322; advances PW-001.

**Decision.** Permanent mutation 290 names
`the keyboard's numbering and the decoder's are the same` as its exact
`Caught::Check` witness. That check directly states the invariant removed by
the mutation and is evaluated only after the translated byte has arrived, so
it neither depends on a harness timeout nor mistakes silence for evidence. A
failed machine boot remains inconclusive unless this exact check is among its
typed failed-check records; the companion decoded-key failure may also occur
but is not required to grade the mutation. The registry contract resolves the
named check in the current machine inventory, and a sweep restarted under the
repaired registry identity must finish with no survivor or inconclusive result
before its qualification record can be promoted.

**Rationale.** The complete sweep on `03b9543` found mutation 290, which stops
the input driver adopting the scan-code numbering the stream proved, still
registered as a sixty-second timeout although both bounded observations
finished and failed `a translated keystroke decodes to the same key` and the
numbering check, the causal pair D-239 records. The stale classification made a
detected defect inconclusive and blocked any complete sweep from passing, and
contradicted D-322's exact-outcome rule. The translated-key check alone is less
direct because decoding or event-delivery defects can cause it, and accepting
any failed check would turn an unrelated red boot into false coverage.
