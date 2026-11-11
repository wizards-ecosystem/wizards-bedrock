# D-446 An admitted self-test wait stops when its clock is revoked

Status: Accepted, 2026-10-26. Qualification OPEN; advances PW-723.

**Decision.** Keep the bounded admission check and share the admitted-deadline
loop through a private helper. Check `is_calibrated` before evaluating the
predicate on every iteration and return false immediately when authority is
lost. Existing callers can then execute their refusal cleanup. The public
clock still freezes on revocation, and no deadline or timing tolerance changes.

The clock-disagreement machine witness computes a future deadline while the
clock is live and uses the real watchdog-disagreement hook; inside revocation,
the shared loop must return false without calling its predicate. The predicate
returns true on its second call so removing the new guard produces a finite,
named assertion failure instead of a host timeout. Retained mutation 2,444
removes that guard and must fail the existing clock-disagreement check in both
profiles.

**Rationale.** `runs_until` checked clock liveness only before admitting a
wait, but the live timer watchdog can revoke the clock afterward, freezing
public time while the fallback scheduler continues, so a condition that stays
false, such as absent input during the held NVMe round, could spin forever
before D-438's command-release cleanup. The defect is in the self-test wait,
not the production scheduler's revocation handling. Fallback scheduler time
would turn scheduling progress into a trusted public deadline, QEMU's host
timeout would leave guest cleanup unproved, and a sentinel-only synthetic
clock would not exercise actual public clock freezing and calibration loss.
