# D-256 Timer cleanup depends on admitted authority, and degraded panics are typed

Status: Accepted, 2026-08-31. Amends D-90, D-209 and D-249; advances PW-122 and PW-533.

**Context.** Returning a typed error from local-timer disarm exposed a valid
degraded path which the earlier infallible wrapper had hidden. With x2APIC
absent, no countdown conversion is ever published, but scheduler cleanup still
asks for an idle timer to be stopped. Treating that unavailable device as a
live-timer contradiction panicked an otherwise responsive degraded kernel.

The degraded runner also reduced every panic to a generic error and checked
only an atomic flag before the serial reader joined. A panic printed in the
settle/kill interval could therefore be logged after the last check and still
be reported as a clean boot. A missing-marker mutation could not distinguish
the intended no-APIC cleanup failure from an unrelated terminal defect.

**Decision.** `TimerError::Unavailable` is benign only while the APIC
countdown conversion remains uncalibrated. Publishing that conversion follows
successful calibration, disarm and one-shot configuration and is never
revoked; after publication, losing the timer is terminal. `StillRunning` and
`InvalidConfiguration` are always terminal. The broader user-clock predicate
is not used because a self-test may revoke that policy after hardware timer
authority exists.

The boot serial reader now retains a panic transcript and whether the exact
no-x2APIC diagnosis preceded it. The degraded observer returns either settled
evidence or that typed panic only after the reader joins. Mutation grading has
a distinct degraded-panic outcome: a green degraded baseline is mandatory,
and only the prior diagnosis plus the exact expected panic is caught. A wrong
panic, missing diagnosis, timeout or build failure is inconclusive.

**Alternatives and evidence.** Ignoring every unavailable timer was rejected
because it hides loss of established hardware authority. Keying cleanup to
user-launch calibration was rejected because that policy is independently
revocable. Treating any degraded panic or missing marker as sufficient was
rejected because an unrelated crash proves nothing about this branch. Host
tests preserve the transcript and reject wrong, unordered and undiagnosed
panics. Removing only the pre-publication exception produced the exact timer
panic after the no-x2APIC diagnosis; shortening either extended-state or user
refusal marker failed on that exact missing marker. All three changes were
restored, the degraded kernel settled, and the timer break remains in the
mutation registry. Current debug and ordinary release boots still gate the
register rows.
