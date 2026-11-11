# D-197 A refused PCI command enablement closes NVMe before a window exists

Status: Accepted, 2026-08-27. Amends D-168 and D-169; advances PW-206.

**Decision.** `libs/pci-config::enable_bus_master` owns a host-testable command
transaction. It reads the original command, returns immediately when memory
decode and bus master are already verified, otherwise writes the preserved
command plus both required bits and reads them back. Any refusal or access loss
after the write attempt triggers an exact original-command restore and another
read. The error distinguishes an access failure or explicit refusal whose
rollback was verified from a restore that could not be verified.

The kernel's `Device` adapts that transaction to the claimed CF8/CFC pair. The
boot sequence can obtain BAR0's validated memory `Window` only from a private
admission function which first completes the command transaction. Failure logs
its exact reason and returns no window; all MMIO mapping, controller register
access, DMA allocation and NVMe commands remain syntactically downstream of the
`Some(Window)` boundary.

Self-test builds can construct a claim-shaped device whose enablement is forced
to fail before any configuration cycle. Passing it through the production
admission function must produce no window and exactly one enable attempt.

**Rationale.** The boot path had logged a refused enablement and continued
through mapping, reset, DMA allocation and commands; a controller without bus
master cannot complete DMA and reports no separate error, so the bounded
timeout described the consequence rather than the cause. Checking only the
write's return value proves that the access mechanism existed, not that a device
accepted the bits. Stopping without rollback avoids host DMA but leaves decode
state dependent on which write failed. Moving the whole rule into `pci-config`
keeps configuration sequencing and failure injection on the host; keeping BAR
admission in the kernel keeps the hardware claim and logging at their real
boundary.

**Alternatives.** Waiting for the inevitable NVMe timeout was rejected because
it allocates and publishes DMA memory to a controller known unable to use it.
Clearing bus master unconditionally on failure was rejected because the
pre-call command is the only state the transaction owns and because firmware
may have supplied a working enablement already. A boot feature which disables
the real QEMU controller was rejected because it would erase every later
storage check; the claim-shaped injected device exercises the same admission
function without perturbing the live controller.

**Consequences.** Host fakes cover the already-enabled, successful enable,
readback refusal, every access-loss edge, exact rollback and unverifiable
rollback cases. Restoring the historical log-and-continue path reddened only
`a failed bus master enable stops before nvme mapping`; the injected claim
performed no configuration access and the check never mapped its fake address.
No unsafe operation was added.

**Current disposition (2026-08-30).** Later PCI authority work moved live DMA
admission to `prepared.admit_bus_master` after verified controller disable and
before `nvme::start`. The injected pre-mapping proof above is historical and
does not close that current ordering. PW-206 is `READY`; its remaining proof is
the causal `a failed bus master admission stops before nvme start` machine red
and restored boot.
