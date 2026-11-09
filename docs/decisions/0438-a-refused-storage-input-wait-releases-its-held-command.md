# D-438 A refused storage-input wait releases its held command

Status: Accepted, 2026-10-23. Qualification OPEN; advances PW-719.

The source-pinned release boot in `pristine-41w5yl95` refused clock
calibration. The delayed-read self-test then entered blocking keyboard reads
after `runs_until` had returned false without polling. Its NVMe command remained
held until the blocked test could resume, so no complete verdict followed.
An ordinary expired key wait could reach the same path. The unchanged retry
completed all 294 release checks; that does not discharge the refusal defect.

The delayed round now uses one finish helper: it evaluates the bounded wait,
consumes at most the first two queued events through a nonblocking self-test
reader only after success, and releases the command hold on every result.
Missing or wrong events remain a failure. The interrupt-admission conjunction
is evaluated without bypassing cleanup. Completion and exact teardown still
run after release. A key-arrival observation is not a reservation of queue
entries, so successful observation alone cannot justify a blocking read.

The existing machine witness also exercises the exact helper with an
uncalibrated ready predicate, an expired calibrated predicate, and ready but
missing input. It observes refusal, absence of reader calls where waiting
failed, and the actual cleared hold flag before independent fallback cleanup.
The live positive round still proves device retention, input progress and
exact completion. Retained defects must make this same named check fail with
a completed verdict; a timeout or compilation failure is not its red.
An unavailable real clock refuses before I/O setup. With a running clock,
failed negative probes still reach the live key round so the ordered host
injection protocol can finish. The retained cleanup defect leaves failed waits
held while preserving successful-wait release; probe fallback clears the
observed defect before live I/O.

No clock tolerance, attempt count, deadline, expected machine count or stack
bound changes. The clock warning is corrected to describe failure to obtain
an agreeing believable pair; the previous text incorrectly implied every
window had been believable and mutually inconsistent. Existing logs preserve
only the last rates, so they do not establish the original refusal's cause.

Alternatives rejected: widening a deadline does not release a held command;
skipping the check hides failed prerequisites; a new Drop owner is unnecessary
for the helper's single exit; and scanning past wrong events would weaken the
existing exact FIFO key witness.
