# D-339 NVMe admission and DMA publication are causal ownership boundaries

Status: Accepted, 2026-09-28. Amends D-197, D-251 and D-278; closes PW-206 and PW-213.

**Decision.** A disabled prepared controller can mint start authority only
after the exact mapped function accepts bus mastering from a fresh `CC.EN = 0`
and `CSTS.RDY = 0` observation. Refusal returns the unchanged disabled
controller. Its queues and mapping are retired before the PCI command and
function leases, selection continues only after the exact frame, claim and
mapping baseline is restored, and any uncertain retirement remains
quarantined and stops selection. No refusal reaches `nvme::start` or a
submission doorbell.

Pure DMA ownership follows one linear `Idle -> Reserved -> InFlight`
transaction. Preparation may cancel only `Reserved`; matching completion may
retire only `InFlight`. `publish_with` changes to `InFlight` before invoking
its sole callback, and that callback receives one borrowed, non-`Copy`,
non-`Clone` `DmaPublication` which the register adapter consumes when ringing
the submission doorbell. `DmaState` is likewise neither `Copy` nor `Clone`.

**Rationale.** D-197's pre-mapping refusal proof no longer reached the storage
order, in which the driver disables and prepares a controller and maps its
exact BAR before asking it to admit bus mastering; a refusal at that later
door still had to return every owner without reaching start. Separately, a
callback holding publication authority before the transition could ring the
doorbell while cancellation still described the bytes as host-owned, and
copyable state or authority could recreate that ambiguity. On 2026-09-28
mutation 56 failed only `a failed bus master admission stops before nvme
start`, and mutations 710, 711, 830 and 1,179 through 1,181 each failed the
exact transition test, completion assertion or linear-authority contract.

**Alternatives.** Treating an error log as closure was rejected because the
defect was continuing after the log. Reusing the pre-mapping witness was
rejected because it cannot observe the post-prepare door or its cleanup
owners. Moving to `InFlight` after the callback was rejected because callback
entry is already the permission to touch hardware. A separate boolean or
copyable proof was rejected because it can diverge from the state whose memory
ownership it claims.
