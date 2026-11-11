# D-383 VT-d revocation drains older DMA before returning frame ownership

Status: Accepted, 2026-10-10. Amends D-357 through D-360 and D-369; closes PW-591 and Codex Security occurrence `occ_a1a268487e204fdeb7611fcf`.

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

Unit and source contracts bound the register interface to major versions 1
through 5 and its enforcement in preflight, require drained invalidation at
both `unmap_segment` and `rollback_range`, require the write-buffer flush before
a global drain command, require global actual granularity for drained and
ordinary invalidation, and preserve ordinary invalidation on the successful
mapping path. The live unmapped-frame check observes that its priming read
completed a DMA drain as well as faulting the later unmapped write without
changing memory. Mutation 1,776 downgrades live unmap to ordinary invalidation
and fails that machine check on the missing drain alone. Mutations 1,777
through 1,779 downgrade rollback, omit the flush and issue a plain global
command; mutations 1,780 through 1,784 widen the version bound, bypass its
production gate, and discard the drained-IOTLB, context-cache and
ordinary-IOTLB actual-global readbacks. Each fails its exact witness. Mutation
1,544 remains the distinct witness for omitting unmap invalidation altogether.

**Rationale.** The NVMe completion path had treated a matching completion as
permission to unmap the transfer buffer and return its frames, and both that
unmap and `map_range`'s partial-publication rollback invalidated without Drain
Reads or Drain Writes. A hostile endpoint can post the matching completion
while an older DMA write is in flight, and a plain invalidation stops later
translations without proving the older request finished, so the allocator could
hand the frame to a new owner before that write lands. Version 6 or later
ignores register invalidation, and a cleared command bit alone can describe an
ignored request. Draining every invalidation would add avoidable latency to
successful publication, and fencing BME around each transfer would serialize
the controller; after the leaves are clear, the architected drained
invalidation orders older requests and later requests fault. Trusting a
completion as a hardware drain is the hostile-device assumption this boundary
must not make. Supporting major version 6 or later through the already owned
invalidation queue is future work; silently issuing architecturally ignored
register commands is not a fallback.
