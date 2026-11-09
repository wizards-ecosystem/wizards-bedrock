# D-332 Framebuffer scrolling keeps checked spans and interrupt progress

Status: Accepted, 2026-09-28. Amends D-48, D-137, D-249 and D-329; closes PW-011.

**Context.** Earlier debug boots spent more than a minute after the
interrupt-progress scroll check, and an attempted speedup copied framebuffer
rows outside checked MMIO. D-249 replaced that shortcut with pure checked row
arithmetic and volatile publication, while D-329 bound the complete source
chain. Closure still required current evidence that padded rows remain exact,
cursor publication remains visible, console work does not exclude interrupts
and the restored debug path completes repeatedly within the gate deadline.

**Decision.** `FramebufferLayout` alone computes each upward row-copy plan.
The plan distinguishes visible words from padded pitch, admits only upward
movement and validates both source and destination against the complete shadow
before iteration. Each row publishes its visible words through one checked
MMIO span and commits the same words to the shadow afterward. Console entry
does not disable interrupts around rendering. A line write hides the old cursor
before scrolling and publishes the new cursor after the newline completes.

**Evidence.** On 2026-09-28 all 41 `pixel` tests and its doctest passed, as did
the exact checked-framebuffer source contract. Retained mutations 841 through
844 were applied one at a time and restored byte-for-byte. Publishing stride
words instead of visible words failed
`row_copy_uses_padded_starts_without_publishing_padding` with five published
words against three expected. Inverting upward admission failed
`row_copy_refuses_downward_and_extreme_empty_ranges`; deleting the source bound
made `row_copy_refuses_a_short_shadow_without_side_effects` reach a slice end of
14 in a 13-word shadow; and advancing by visible width instead of pitch failed
`every_small_row_copy_stays_within_its_padded_layout` with offset 4 against 8.

Retained mutation 188 omitted the new cursor publication. Debug QEMU reported
`a scroll moves one visible cursor` as its only failure, passed 261/262 and
recorded `selftest: longest console scroll 367938646 ns`. Retained mutation 189
disabled interrupts around console work. It reported
`a console scroll does not block interrupts` as its only failure, passed
261/262 and recorded `selftest: longest console scroll 150758765 ns`. Each
mutation was restored before the next boot. Two consecutive restored debug
boots then returned in 8.9 and 8.1 seconds, passed 262/262 and recorded exact
longest-scroll values of 370250537 ns and 363899047 ns. The emulated nanosecond
values are regression observations, not native performance claims.

**Alternatives.** Restoring a raw bulk copy was rejected because it bypasses
the checked volatile device boundary and can publish padding. Disabling
interrupts around the console was rejected because rendering latency would
become interrupt latency. A single restored boot was rejected because the
original defect was intermittent enough to demand consecutive current runs.

**2026-10-17 witness correction.** A current CI boot completed its scroll in
6,538,992 emulated ns and sampled the armed APIC timer before QEMU injected it,
so the immediate post-MMIO expiry read failed despite two immediate restored
boots passing the same check. The witness now records scroll duration before a
bounded wait for the real timer expiry, with both the wait and its observation
still inside the same live console guard. Mutation 189's outer interrupt
exclusion therefore cannot deliver the expiry until after the result is fixed
as false, while ordinary execution no longer relies on framebuffer work taking
long enough to schedule a virtual interrupt. The unrelated runnable helper was
removed; later APIC checks retain their count-sensitive self-interrupt oracle.
Applied mutation 189 then failed only `a console scroll does not block
interrupts` at 289/290 with a 145,660,393 ns scroll. Exact restoration passed
two consecutive debug boots at 290/290 with 154,644,248 ns and 156,624,274 ns
scrolls, then passed release at 290/290 with a 4,018,112 ns scroll.
