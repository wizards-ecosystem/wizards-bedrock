# D-264 A child is complete only after bounded exit and capture

Status: Accepted, 2026-08-31. Amends D-134 and D-244; advances PW-019 and PW-022.

**Context.** A caller deadline did not bound a later blocking `wait`, and pipe
capture could wait forever on an inherited writer. Mutation grading could then
call a timeout caught even when termination failed, or leak the first capture
file when reserving the second stream failed.

**Decision.** Qualified host processes own one direct child. They poll through
the caller deadline, attempt termination on failure, and poll reap through one
named additive deadline; no control interface exposes a blocking wait. Stdout
and stderr are exclusive regular files under one rollback reservation and share
one caller-selected byte cap. Collection reads the retained handles only after
an exit was observed. Kill refusal, a still-live child, capture overflow or
partial reservation is an error or inconclusive mutation outcome, never a red
proof.

**Alternatives and current evidence.** Extending the original timeout was
rejected because it leaves an unbounded operation after the extension. Pipes
were rejected because descendants can retain their ends. Parked, noisy,
delayed-exit, exit/kill-race and second-stream fixtures exercise each state, and
the exact mutations name those host tests. This owns only the direct child;
delegated process-tree containment remains PW-013's trigger.

**Current disposition (2026-08-31).** PW-019 and PW-022 are `DONE`: bounded
exit, capture, cleanup and partial-reservation rollback have exact host and
mutation witnesses.

**Amended by D-347.** On Windows the delegated tree this record left to PW-013
is contained: every bounded command runs in a job object that ends with it.
