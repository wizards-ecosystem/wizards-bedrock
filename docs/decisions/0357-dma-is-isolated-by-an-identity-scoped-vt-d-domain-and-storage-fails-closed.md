# D-357 DMA is isolated by an identity-scoped VT-d domain, and storage fails closed without one

Status: Accepted, 2026-09-29.

**Decision.** The storage controller runs behind an Intel VT-d DMA-remapping
domain, and the domain is a precondition of bus mastering. The second-level
tables identity-map only the frames a live `DmaSegment` owns: IOVA equals host
physical for a mapped frame, and every other address faults. Frames are mapped
before publication and unmapped, with the unit's cache invalidated, only after
the existing lifecycle proves the frame device-quiescent, so the reachable set is
exactly the published-and-not-retired set. `admit_bus_master` requires a
`DmaDomain` proof for the exact function, so no path reaches a doorbell without
the fence. If firmware reports no DMAR, no unit governs the function, or the unit
cannot be taken over, storage does not start and the boot says
`storage: no DMA isolation`. If firmware left translation enabled, the kernel
quiesces the unit and installs its own root before re-enabling it, rather than
riding firmware's tables. The tables the unit walks are kernel-owned frames never
mapped to any device, so the device cannot reach what constrains it.

**Scope.** DMA remapping only. Interrupt remapping (PW-710) and AMD-Vi (PW-711)
are trigger rows carrying the surfaces their arrival opens. Per-segment IOVA
translation that also hides the physical map (PW-712) is a further strengthening
the reach guarantee does not require, because identity-mapped-and-scoped already
faults every access outside the live set. The QEMU unit runs `intremap=off`, and
the qualified hosts present Intel units.

**Rationale.** A bus-mastering controller reads and writes physical memory
outside the page tables. The storage phase's DMA properties (PW-200, PW-206,
PW-213, D-339) govern when a frame is device-owned, not which addresses the
controller may name, so without an IOMMU a wedged, buggy or reprogrammed
controller can reach any frame; the census (017) recorded the DMA lifetime but
not the DMA reach.

**Verification.** The table parse (`libs/acpi-tables`) and register encoding
(`libs/vtd`) are host arithmetic that refuses malformed DMAR structures, an
inverted reserved region and a duplicate unit and walks the capability and
width arithmetic exhaustively. Boots check that translation is
active before the first doorbell, that storage and the filesystem read through
the unit, that a read into an unmapped address records a fault and changes no
memory, and that an unmap invalidates before the frame returns. Mutations keep
a skipped invalidation, an extra mapped page, bus mastering before the domain,
translation left off and a context for the wrong function.

**Alternatives.** Trusting the controller not to DMA outside its buffers is the
status quo the census exposed as unbacked. Per-segment IOVA translation was
deferred rather than adopted, because the reach guarantee does not require hiding
the map and the extra allocator and encoding would be boot-verified surface with
no current caller. Leaving isolation optional, so storage runs without a unit
when none is present, was rejected: an absent IOMMU is exactly the case where the
controller is unconstrained, so it must fail closed.

**Amended by D-358.** The verification paragraph above names invalidation
descriptors; the shipped unit invalidates through the Context Command and IOTLB
registers, and the queued encoder was removed as speculative API. The unit is
taken over after `nvme::prepare` and released after the candidate's lease, and a
unit whose page walks are not coherent with the processor caches is served by
flushing its table lines rather than refused.
