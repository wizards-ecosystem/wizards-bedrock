# D-383 VT-d revocation drains older DMA before returning frame ownership

Status: Accepted (2026-10-10), amends D-357 through D-360 and D-369; closes PW-591 and Codex Security occurrence `occ_a1a268487e204fdeb7611fcf`

**Context.** The NVMe completion path treated a matching completion as permission
to remove the transfer buffer from the VT-d domain and return its frames. Both
that live unmap and `map_range`'s partial-publication rollback cleared their
second-level leaves and completed a global IOTLB invalidation without setting
Drain Reads or Drain Writes. A hostile endpoint can post the matching completion
while an older DMA write remains in flight. The plain invalidation stops later
translations but does not prove the older request finished, so the allocator
could hand the frame to a new owner before that write lands. The same ownership
error was possible after a partially published map was rolled back.

**Decision.** Preflight admits the register invalidation interface only for
architectural major versions 1 through 5; version zero is malformed and version
6 or later requires queued invalidation, which this owner does not yet use.
Every live VT-d revocation clears every affected leaf, preserves the first
removal error while attempting the whole range, performs any capability-required
write-buffer flush, and then issues one global IOTLB invalidation with both read-
and write-drain bits set. It waits for bounded completion and accepts it only
when the IOTLB Actual Invalidation Granularity reports global. Context-cache and
ordinary IOTLB invalidations apply the same accepted-global check instead of
treating a cleared command bit as proof that hardware executed the request. Any
leaf, register, flush, ignored request or completion failure keeps the existing
`Retained`/quarantine outcome, so no uncertain frame is recycled. Successful
mapping publication continues to use an ordinary global invalidation: it
publishes reach rather than revoking ownership and therefore does not pay the
drain boundary. Unpublished-table rollback remains local because hardware
cannot walk those tables.

**Proof required.** Unit and source contracts must bound the register interface
to major versions 1 through 5, require that preflight enforces that bound,
require drained invalidation at both `unmap_segment` and `rollback_range`,
require the write-buffer flush before a global drain command, and require the
reported actual granularity to be global for drained and ordinary invalidation.
They must preserve ordinary invalidation on the successful mapping path. The
live unmapped-frame check must observe that its priming read completed a DMA
drain in addition to faulting the later unmapped write without changing memory.
Persistent mutations must independently downgrade live unmap and rollback, omit
the write-buffer flush, replace the drained command with a plain global
invalidation, widen or bypass the version boundary, and accept cleared command
bits without the global-granularity readback.

**Evidence.** The new source contract first failed on the old live unmap, which
contained no `drain_dma` call. The repaired contract and focused compiler gate
pass. Release QEMU passes all 281 checks and records the strengthened
unmapped-frame witness. Mutation 1,776 downgrades live unmap to ordinary
invalidation: the device still faults on its later access, but the missing drain
observation alone makes that exact machine check fail at 280/281. Mutations
1,777 through 1,779 independently downgrade rollback, omit the required flush
and issue a plain global command; each fails the exact source contract before
byte-for-byte restoration. The required post-patch review then found that
version 6 or later ignores register invalidation and that a cleared command bit
alone can describe an ignored request. Mutations 1,780 through 1,784 now widen
the version bound, bypass its production gate, and independently discard the
drained-IOTLB, context-cache and ordinary-IOTLB actual-global readbacks; each
failed its exact unit or source witness before restoration. Mutation 1,544
remains the distinct witness for omitting unmap invalidation altogether. The
closing ordinary gate resolves all 2,040 public API items, passes 750 `xtask`
tests with 10 intentional ignores, verifies all 80 registry crates and runtime
artifacts, passes both self-test images at 281/281, and passes the shipped and
two fail-closed degraded boots.

**Alternatives.** Draining only `unmap_segment` leaves partial-map rollback able
to return frames behind older DMA. Draining every invalidation also covers the
bug but adds avoidable latency to successful publication. Fencing BME around
each transfer would serialize the entire controller and is not needed: after
the leaves are clear, the architected drained invalidation orders older requests
and later requests fault. Trusting a completion as a hardware drain is precisely
the hostile-device assumption this boundary must not make. Supporting major
version 6 or later through the already owned invalidation queue is future work;
silently issuing architecturally ignored register commands is not a fallback.
