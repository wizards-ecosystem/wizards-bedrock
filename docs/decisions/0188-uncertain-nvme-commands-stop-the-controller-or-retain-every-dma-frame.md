# D-188 Uncertain NVMe commands stop the controller or retain every DMA frame

Status: Accepted, 2026-08-27. Amends D-169 and closes PW-200.

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

**Rationale.** D-169's timeout returned while the command was outstanding, and
the Identify buffer, partial queue frames, a failed caller's read buffer or the
Admin queues could then drop while the enabled device still held their physical
addresses; Rust ownership says nothing about a device doing DMA on its own. NVM
Express Base 2.2 sections 3.4.5 and 3.7.2.1 give the provable boundary: clearing
`CC.EN` initiates Controller Reset, and observing `CSTS.RDY = 0` means the
outstanding command can no longer be processed, with the I/O queues deleted and
the Admin queues reset. Writing the bit without observing readiness is not that
proof. Permanent retention is the only sound fallback when the proof cannot be
obtained: reusing even one frame would let the device write into an unrelated
future owner. A dedicated owner makes the fallback unrepresentable as an
ordinary `Segment` rather than depending on every error site remembering not to
drop one.

**Alternatives.** Waiting longer still leaves a finite timeout with the same
edge. Abort is an Admin command that itself needs live queues and a completion,
so it cannot close failures of those objects. Reset without waiting for
`RDY = 0` assumes the exact asynchronous transition at issue. Leaking all NVMe
memory on every error was safe but would discard the specification's positive
quiescence proof and make recoverable completed-command errors unnecessarily
permanent.

**Consequences.** A delayed fake device proves that a failed stop remains
unrecyclable even after a late completion, while a verified stop and a matching
completion permit recycling. QEMU withholds a deliberately retained frame and
runs the production timeout arm after every storage/VFS check; the latter
submits a real read, skips polling, resets the controller and observes
`RDY = 0` before the buffer can return. Three persistent mutations preserve
those checks' exact failures.
