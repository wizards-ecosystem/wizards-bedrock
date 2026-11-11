# D-332 Framebuffer scrolling keeps checked spans and interrupt progress

Status: Accepted, 2026-09-28. Amends D-48, D-137, D-249 and D-329; closes PW-011.

**Decision.** `FramebufferLayout` alone computes each upward row-copy plan.
The plan distinguishes visible words from padded pitch, admits only upward
movement and validates both source and destination against the complete shadow
before iteration. Each row publishes its visible words through one checked
MMIO span and commits the same words to the shadow afterward. Console entry
does not disable interrupts around rendering. A line write hides the old cursor
before scrolling and publishes the new cursor after the newline completes.

**Rationale.** Earlier debug boots spent more than a minute after the
interrupt-progress scroll check, and an attempted speedup copied framebuffer
rows outside checked MMIO. D-249 replaced that shortcut with checked row
arithmetic and volatile publication and D-329 bound the source chain; closure
needed current evidence that padded rows stay exact, the cursor stays visible,
console work does not exclude interrupts and the restored debug path completes
repeatedly within the gate deadline. Retained mutations 841 through 844 each
fail only their named `pixel` row-plan test, mutation 188 omits the new cursor
and fails only `a scroll moves one visible cursor`, and mutation 189 disables
interrupts around console work and fails only
`a console scroll does not block interrupts`.

**Alternatives.** Restoring a raw bulk copy was rejected because it bypasses
the checked volatile device boundary and can publish padding. Disabling
interrupts around the console was rejected because rendering latency would
become interrupt latency. A single restored boot was rejected because the
original defect was intermittent enough to demand consecutive current runs.

**2026-10-17 witness correction.** A CI boot sampled the armed APIC timer
before QEMU injected it, so the immediate post-MMIO expiry read failed. The
witness now records scroll duration before a bounded wait for the real timer
expiry, with both the wait and its observation inside the same live console
guard, so mutation 189's outer exclusion still cannot deliver the expiry before
the result is fixed as false, and ordinary execution no longer relies on
framebuffer work taking long enough to schedule a virtual interrupt. Later
APIC checks retain their count-sensitive self-interrupt oracle.
