# D-394 Mutation witnesses follow the current causal verdict

Status: Accepted, 2026-10-12. Amends D-239 and D-322; advances PW-001.

**Context.** The complete mutation sweep on `03b9543` reached permanent
mutation 290, which prevents the input driver from adopting the scan-code
numbering proved by the arriving stream. Its registry still expected the
machine to time out after sixty seconds. Both bounded observations instead
finished normally and failed the same two existing checks:
`a translated keystroke decodes to the same key` and
`the keyboard's numbering and the decoder's are the same`. D-239 already
records that exact pair as the causal result of deciding the numbering without
adopting it. The stale timeout classification therefore made a detected defect
inconclusive and guaranteed that an otherwise complete sweep could not publish
a qualification pass.

**Decision.** Permanent mutation 290 names
`the keyboard's numbering and the decoder's are the same` as its exact
`Caught::Check` witness. That check directly states the invariant removed by
the mutation and is evaluated only after the translated byte has arrived, so
it neither depends on a harness timeout nor mistakes silence for evidence. A
failed machine boot remains inconclusive unless this exact check is among its
typed failed-check records; the companion decoded-key failure may also occur
but is not required to grade the mutation.

**Proof required.** The registry contract must resolve the named check in the
current machine inventory. Applying mutation 290 must complete a release
self-test boot, fail the named numbering check, restore the isolated source
byte for byte and record `caught`. The restarted complete sweep must use the
new registry fingerprint and finish with no survivor or inconclusive result
before its qualification record can be promoted.

**Alternatives.** Keeping the timeout would contradict two repeatable terminal
verdicts and D-322's exact-outcome rule. Naming only the translated-key check is
less direct because that symptom can arise from decoding or event-delivery
defects. Accepting either arbitrary failed check would turn an unrelated red
boot into false mutation coverage.
