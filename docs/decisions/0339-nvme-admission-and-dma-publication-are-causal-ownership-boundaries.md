# D-339 NVMe admission and DMA publication are causal ownership boundaries

Status: Accepted, 2026-09-28. Amends D-197, D-251 and D-278; closes PW-206 and PW-213.

**Context.** D-197's original pre-mapping refusal proof no longer reached the
current storage order. The live driver now disables and prepares a controller,
maps its exact BAR and only then asks that prepared controller to admit bus
mastering. A refusal at that later door still needed to prove that it returns
every mapping, queue, command and function owner without reaching controller
start or a submission doorbell.

Separately, distinguishing a reserved DMA command from an in-flight command
did not by itself prove that the transition preceded the device handoff. If
the callback received publication authority first, it could ring the doorbell
while cancellation still described the bytes as host-owned. Copyable state or
publication authority could independently recreate the same ambiguity.

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

**Evidence.** On 2026-09-28 mutation 56 converted the forced post-disable PCI
refusal into success. The first controller falsely logged bus-master admission
and reached start, while the later real controller still completed its block
read; only `a failed bus master admission stops before nvme start` failed, at
261/262, with no terminal panic or secondary failed check. After exact
restoration the forced refusal logged its typed command error, retired
candidate one cleanly, and candidate two enabled bus mastering only after
verified disable, started with eight-entry queues, 133,120 blocks of 512 bytes
and four-byte-spaced doorbells, then read block zero.

Mutation 710 invoked the publication callback before changing ownership and
the exact transition test observed `Some(false)` instead of `Some(true)`.
Mutation 711 allowed completion of a merely reserved command and failed
`assertion failed: !state.complete()`. Mutations 830 and 1,179 through 1,181
made `DmaState` or `DmaPublication<'static>` implement `Clone` or `Copy`; each
made its hostile compiler probe succeed and failed only the exact linear-
authority contract. Every mutation was restored byte-exact before the next.

Exact restoration passed all 78 `pci-config` unit tests and two doctests, all
34 `nvme` unit tests and two doctests, all 15 DMA authority contracts, the
exact PCI and linear-publication contracts, and all 1,412 mutation
applicability and named-check registrations. Restored release QEMU passed
262/262 with the causal refusal, exact retirement and later real NVMe path.

**Alternatives.** Treating an error log as closure was rejected because the
defect was continuing after the log. Reusing the pre-mapping witness was
rejected because it cannot observe the current post-prepare door or its cleanup
owners. Moving to `InFlight` after the callback was rejected because callback
entry is already the permission to touch hardware. A separate boolean or
copyable proof was rejected because it can diverge from the state whose memory
ownership it claims.
