# D-388 Firmware-reserved DMA regions carry no mapping authority

Status: Accepted (2026-10-11), amends D-360

**Context.** D-360 screened a device's ACPI DMAR Reserved Memory Region
Reporting (RMRR) entries against the boot memory map, then identity-mapped the
entries that fell wholly in a device hole. That map distinguishes allocator
memory from non-RAM; it does not distinguish a device-owned buffer from VT-d,
APIC, PCI configuration or another security-critical MMIO window. Hostile or
mistaken firmware could therefore name a critical register page as an RMRR and
give the selected bus master read/write access to it before translation was
enabled. Checking only the selected DRHD's register page would leave the same
failure for every other critical physical window.

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

**Proof required.** A source contract must bind all three properties: `Unit`
has no reserved-region authority, `look_up` rejects the first applicable RMRR,
and every VT-d establishment or retarget call receives an empty list. The
machine check must accept an empty list and refuse both a live device-hole page
and a usable-RAM page through the exact production predicate. Permanent
mutation 1,589 now consumes the first entry but reports the iterator empty; it
must redden the source contract and the machine witness before byte-for-byte
restoration. Release and debug boots, the no-DMAR degraded boot and the complete
CI gate must pass afterward.

**Alternatives.** Screening only allocator-owned memory repeats D-360's gap.
Rejecting overlap with every DRHD register page closes the demonstrated VT-d
variant but not other critical MMIO. Trusting firmware to declare only benign
device buffers contradicts the hostile-firmware boundary. Supporting RMRRs
later requires one complete, typed physical-ownership ledger rather than a
growing denylist of known register ranges.
