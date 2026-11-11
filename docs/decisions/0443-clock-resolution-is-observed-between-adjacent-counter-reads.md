# D-443 Clock resolution is observed between adjacent counter reads

Status: Accepted, 2026-10-26. Qualification OPEN; advances PW-721.

**Decision.** Keep the existing bounded public-clock sweep for monotonicity
and record whether that clock actually advances. Observe resolution separately
with one thousand adjacent timestamp-read pairs. Both samples use the same
conversion helper as `raw_now_ns`, which retains its existing order: load the
`Timebase` before reading the timestamp, then convert relative to the origin.
The resolution witness requires a live calibration, observed public progress
and a strictly positive converted step below one thousand nanoseconds. A
backward adjacent pair also fails monotonicity. The witness neither widens
that threshold nor retries failed boots.

Mutation 2,441 quantizes the shared conversion to microseconds; mutation
2,442 freezes the public sweep while the counter pairs still advance. Each
must fail the existing named resolution check in release and debug before
restoration. This repairs a witness; it makes no new claim about calibration,
hardware qualification or the cause of older failed runs.

**Rationale.** The debug witness had failed its sub-microsecond check while
monotonicity passed, because its minimum step came from public calls separated
by loop bookkeeping and deadline conversion, so a large step did not establish
a coarse clock. The existing capture does not identify a cause for each past
failure, and no historical timing estimate is adopted as evidence. Raising the
threshold would change the clock promise, more samples from the same loop
still include its cost, and testing only the arithmetic would observe no
running counter or public progress.
