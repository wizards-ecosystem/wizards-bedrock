# D-251 Hardware observations mint exact, non-derivable authority

Status: Accepted, 2026-08-30. Amends D-188, D-205 and D-249; advances PW-130 and PW-213.

**Context.** Pure `nvme::DmaState` claimed whether hardware was quiesced, and
the boot module converted a forgeable enum into permission to enable and route
external interrupts. Neither representation tied the permission to the device
whose register state supplied it.

**Decision.** Pure NVMe state records only reservation, publication,
completion and terminal closure. The kernel separately records active,
quiesced or retained disposition. Only `stop_controller` may produce
`ControllerStopped<'_>`, after observing `CC.EN = 0` and `CSTS.RDY = 0`; that
value borrows the exact controller mapping and exact DMA state, and consuming
it performs the quiescence transition. Failure closes publication and retains
every reachable run.

I/O APIC preparation similarly produces one opaque
`ExternalControllersMasked`. Its fields are private, it has no cloning or
default construction, interrupt enable borrows it, and routing consumes it
only after rechecking the live interrupt flag. An unproven inventory cannot
mint it.

**Alternatives and evidence.** A boolean or public enum was rejected because
the caller could restate the conclusion. Keeping hardware disposition in the
pure crate was rejected because host logic cannot observe the register. Thirty
NVMe tests, two doctests, five DMA contracts, routing contracts and a 247-check
release boot passed. Cross-controller stop evidence, wrong readiness, a second
mask mint, a default-derived mask, bypassed live-IF checks and every disposition
inversion were each observed red before restoration.
