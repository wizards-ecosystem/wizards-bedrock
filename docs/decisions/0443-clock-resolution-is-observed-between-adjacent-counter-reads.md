# D-443 Clock resolution is observed between adjacent counter reads

Status: Accepted, 2026-10-26. Qualification OPEN; advances PW-721.

**Context.** The debug clock witness has failed its sub-microsecond check
while its monotonic check passed. Its minimum step came from public clock
calls separated by loop bookkeeping and deadline conversion. That interval
includes the witness's execution cost, so a large step does not establish a
coarse clock. The existing capture does not identify a cause for each past
failure, and no historical timing estimate is adopted as evidence.

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
restoration. The restored paired boots, ordinary gate and complete current
sweep remain required. The machine-check count is unchanged. This repairs a
witness; it makes no new claim about calibration, hardware qualification or the cause of
older failed runs.

**Alternatives.** Raising the threshold changes the clock promise. Retrying a
failed witness can hide a regression. Taking more samples from the same loop
still includes its execution cost. Testing only the conversion arithmetic
would not observe a running counter or public-clock progress. The separate
observations retain both obligations without making loop speed the resolution
oracle.
