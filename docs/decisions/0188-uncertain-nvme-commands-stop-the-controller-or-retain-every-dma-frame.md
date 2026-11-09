# D-188 Uncertain NVMe commands stop the controller or retain every DMA frame

Status: Accepted (2026-08-27), amends D-169 and closes PW-200

**Context.** D-169 bounded every polled command, but its timeout returned while
the command was still outstanding. The local Identify buffer then dropped;
partial I/O-queue construction dropped queue frames; a failed caller could
drop its read buffer; and a failed bring-up dropped the controller's Admin
queues while the enabled device still held every physical address. A wrong
completion identifier and a queue-memory or MMIO error after ringing the
doorbell had the same ambiguity. Rust ownership covered the host but said
nothing about an independent DMA agent.

NVM Express Base 2.2 sections 3.4.5 and 3.7.2.1 provide the boundary the driver
can prove. Clearing `CC.EN` initiates Controller Reset; observing `CSTS.RDY = 0`
means the outstanding command can no longer be processed. The reset also
deletes the I/O queues, resets the Admin queues and brings the controller idle.
Writing the bit without observing readiness is not that proof.

**Decision.** Pure `nvme::DmaState` records `Idle`, `InFlight`, `Quiesced` and
`Retained` ownership. The driver enters `InFlight` immediately before the
submission doorbell write may publish the command. Only its matching completion
returns to `Idle`; another identifier, a poll failure and timeout remain
outstanding. Every ambiguous exit clears `CC.EN` and waits for `CSTS.RDY = 0`.
Fatal status may end an enable wait, but it cannot substitute for the cleared
ready bit while disabling.

`wost::mm::DmaSegment` is the safe memory owner at that boundary. It exposes a
physical address and host reads/writes while ordinary ownership is certain. If
reset or its observation fails, `retain` removes all safe access and forgets
the underlying `Segment`, permanently withholding those frames from the
allocator. Cleanup retains both Admin queues, both optional I/O queues and the
current command buffer together. A later completion cannot reverse that state.
The I/O pair is moved into `Controller` before its first Create Queue command,
and `Controller::drop` applies the same stop-or-retain rule to partial bring-up
and configured queues.

**Rationale.** Reset plus the readiness observation is the controller's own
proof that DMA has ceased. Permanent retention is the only sound fallback when
that proof cannot be obtained: reusing even one frame would let the device
write into an unrelated future owner. A dedicated owner makes the fallback
unrepresentable as an ordinary `Segment` rather than depending on every error
site remembering not to drop one.

**Alternatives.** Returning directly was the defect. Waiting longer still
leaves a finite timeout with the same edge. Abort is an Admin command that
itself needs live queues and a completion, so it cannot close failures of those
objects. Reset without waiting for `RDY = 0` assumes the exact asynchronous
transition at issue. Leaking all NVMe memory on every error was safe but would
discard the specification's positive quiescence proof and make recoverable
completed-command errors unnecessarily permanent.

**Consequences.** A delayed fake device proves that a failed stop remains
unrecyclable even after a late completion, while a verified stop and a matching
completion permit recycling. QEMU withholds a deliberately retained frame and
runs the production timeout arm after every storage/VFS check; the latter
submits a real read, skips polling, resets the controller and observes
`RDY = 0` before the buffer can return. The three checks were each broken and
observed red, and three persistent mutations preserve those exact failures.
The image now carries 194 machine checks and the register 93 mutations.
