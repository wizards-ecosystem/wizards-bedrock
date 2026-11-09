# D-446 An admitted self-test wait stops when its clock is revoked

Status: Accepted (2026-10-26), qualification OPEN; advances PW-723

**Context.** `runs_until` checks clock liveness before admitting a wait but
then compares its deadline against public time without rechecking calibration.
The live timer watchdog can revoke that clock after admission. Public time
then freezes while the fallback scheduler continues. A condition that stays
false, such as absent input during the held NVMe round, can therefore spin
forever before D-438's command-release cleanup is reached. This defect is in
the self-test wait, not the production scheduler's revocation handling.

**Decision.** Keep the bounded admission check and share the admitted-deadline
loop through a private helper. Check `is_calibrated` before evaluating the
predicate on every iteration and return false immediately when authority is
lost. Existing callers can then execute their refusal cleanup. The public
clock still freezes on revocation, and no deadline or timing tolerance changes.

Extend the existing clock-disagreement machine witness using a future deadline
computed while the clock is live and the real watchdog-disagreement hook.
Inside revocation, the shared loop must return false without calling its
predicate. The predicate returns true on its second call so removing the new
guard produces a finite, named assertion failure instead of a host timeout.
Retained mutation 2,444 removes that guard. It must fail the existing clock
disagreement check in both profiles, followed by restored passing boots and
the ordinary gate. The expected machine-check count remains unchanged.

**Alternatives.** Trusting the initial admission misses later revocation.
Using fallback scheduler time would turn scheduling progress into a trusted
public deadline. Depending on QEMU's host timeout would leave guest cleanup
unproved. A sentinel-only synthetic clock would not exercise actual public
clock freezing and calibration loss, so the witness uses the live revocation
path instead.
