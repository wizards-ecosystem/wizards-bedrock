# D-388 Firmware-reserved DMA regions carry no mapping authority

Status: Accepted, 2026-10-11. Amends D-360.

**Decision.** No firmware RMRR is supported by the release kernel. After the
sealed topology proves complete requester coverage, `iommu::look_up` asks the
validated DMAR whether any RMRR applies to the candidate and refuses the whole
unit at the first one. `Unit` carries no reserved-region array or count, and
both fresh establishment and parked-owner retargeting pass an empty reserved
list to `Vtd`. Thus firmware bytes can select a supported remapping unit but
cannot become DMA mapping authority. The interrupt-only owner follows the same
rule. The now-unreachable `wost::mm::reserved_region_is_eligible` door is
removed rather than left as a public kernel API; the pure `memory-map`
classifier and its arithmetic tests remain historical evidence for D-360, not
runtime authority. A machine that needs an RMRR remains fail-closed with
`storage: no DMA isolation` until a future typed physical ledger can prove the
region is a device buffer and disjoint from every kernel, firmware and control
window. QEMU's qualified DMAR names no RMRR, so this compatibility boundary
does not remove the release path.

A source contract binds all three properties: `Unit` has no reserved-region
authority, `look_up` rejects the first applicable RMRR, and every VT-d
establishment or retarget call receives an empty list. The machine check
accepts an empty list and refuses both a live device-hole page and a usable-RAM
page through the exact production predicate. Mutation 1,589 consumes the first
entry but reports the iterator empty and reddens the source contract and the
machine witness.

**Rationale.** D-360 identity-mapped RMRRs that fell wholly in a boot-map
device hole, but that map does not distinguish a device-owned buffer from VT-d,
APIC, PCI configuration or another critical MMIO window, so hostile or mistaken
firmware could give the selected bus master access to a register page before
translation was enabled. Rejecting overlap with every DRHD register page would
close only the demonstrated variant, and trusting firmware to declare benign
buffers contradicts the hostile-firmware boundary. Supporting RMRRs later
requires one complete, typed physical-ownership ledger rather than a growing
denylist of known register ranges.
