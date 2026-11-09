# D-435 QEMU cleanup and QMP command waits have finite failure paths

Status: Accepted (2026-10-23), qualification OPEN; amends D-227 and D-346; advances PW-716

The self-test observer held an ordinary child handle, whose Drop does not stop
QEMU. Early polling/setup errors could skip termination. Timeout cleanup could
join an injector before stopping its pipe peer, and synchronous QMP writes
could block before reaching the reply deadline. Shipped/degraded serial joins
also lacked a finite finish boundary; an initial status-poll error could skip
the termination attempt.

Self-test acquires a private child owner immediately after spawn. Its cleanup
cancels injection, attempts bounded stop/reap, closes serial, then collects
workers under one shared finite deadline. If no stop was attempted, the owner's
Drop cancels and attempts that bounded stop after an unexpected return or unwind;
it does not collect workers or retry an already-attempted stop. Shipped and
degraded boots retain centralized cleanup and now bound their serial joins
after the stop attempt. An initial poll failure still reaches termination and
remains in the returned error. Observation, serial, worker and stop errors are
combined rather than discarding one. An unfinished worker is reported and
detached, never synchronously joined. OS refusal remains an error.

Timeout diagnostics run only with an already-finished injector and in a
separate worker with bounded waits for its completion. The original exact
timeout classification is formed before cleanup results are combined. Only a
clean observation timeout retains
the exact text recognized by the mutation harness; cleanup failure cannot
masquerade as the registered timeout witness.

Each QMP session owns one persistent writer and capacity-one request and
acknowledgement queues. Public command text is limited to 64 KiB and encoded
requests to 1 MiB. Submission and acknowledgement waits check cancellation
and the command deadline; the worker refuses expired or abandoned queued work.
An exchange poisons the session before starting and clears that state only
after a complete write and a matched successful reply. Any incomplete exchange
permanently refuses reuse and closes the producer, so late replies or write
acknowledgements cannot answer another command.

A blocked OS write may outlive the caller's bounded error until the owned
child's pipe closes. No unsafe write cancellation or pump-join guarantee is
claimed. Termination refusal cannot produce PASS. A thread per command and
unbounded queues were rejected because they create unbounded resident work;
small production commands alone cannot prove an OS pipe-capacity bound.

Causal fake peers cover blocked write/flush, cancellation, queue pressure,
deadline expiry, failed writes, late completion and successful reuse. Child
fixtures cover early error, unwind, kill/reap refusal and combined outcomes;
source contracts bind actual production ownership and stop-before-join paths.
API review names the exact private `QemuLifetime` Drop type and `new` producer,
plus the `ControlledWriter` and `CountedWriter` fixture Drop types; neither
fixture has a new reviewed producer. Existing mutation 813 retains its
refused-kill, still-live defect by replacing `Ok(false) => return termination`
with success in the revised termination helper, and keeps its original causal
child-stop witness.
Exact mutation reds, restored focused passes, full CI and current machine
boots remain pending at this decision's implementation checkpoint.

The hostile-surface census now names the actual transports: QMP on owned
standard streams, controlled-boot serial on a private Windows named pipe or
Unix socket with OS backpressure, degraded serial on stdout, and the current
`nvme-stack v2` leaf record.
