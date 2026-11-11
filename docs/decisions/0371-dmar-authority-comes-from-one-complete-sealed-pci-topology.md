# D-371 DMAR authority comes from one complete sealed PCI topology

Status: Accepted, 2026-10-09. Amends D-358 and D-370; closes PW-581.

**Decision.** Immediately after PCI configuration access is acquired, and
before ordinary enumeration can select a storage candidate, the kernel probes
all 256 buses, 32 devices and eight functions independently. It does not trust
multifunction hints or a bridge-derived reachability walk for this authority.
The resulting fixed `PciTopology` lives in static storage, not the 32 KiB boot
stack. Every observed bridge must agree in class and header layout, expose bus
numbers whose primary byte is its own bus, forward to a strictly later
secondary bus, and have a nonempty ordered interval. Each non-root bus has one
direct parent; every present non-root function reaches bus zero; each child's
interval is contained by its parent; intersecting intervals are allowed only
when one bridge is an actual ancestor of the other. Any refusal leaves the
snapshot unsealed.

VT-d lookup fails before deriving authority unless that snapshot is sealed.
Bridge ranges, exact requester presence, hierarchy ancestry and RMRR coverage
all query that same immutable snapshot. A hierarchy scope covers its root and
only targets both numerically within the range and proved below the root in the
parent chain. Numeric overlap alone names nothing. This remains a static-boot
contract: the kernel has no hot-plug or bridge-renumbering service, and adding
one must invalidate and reseal this authority before admitting DMA.

DMAR structural validation and defensive scope iteration share one path
grammar. Every scope path has an even byte count, every PCI endpoint and
sub-hierarchy has at least one hop, and every hop encodes a device below 32 and
a function below eight. A malformed scope invalidates the complete DMAR before
publication; an iterator handed unvalidated bytes stops and cannot resume past
them. A well-formed path has no smaller implementation limit than the scope's
one-byte length: every hop is walked, so a valid deep endpoint cannot disappear
and expose a less-specific unit's hierarchy.

**Rationale.** Deep Codex Security scan
`32245094-b8b7-47d1-b6e7-a52554800772` found two remaining ways firmware or
bridge configuration could assign DMA authority to the wrong VT-d unit,
occurrences `occ_4ba847796226668b8378e266` and
`occ_ecda4a10e95e394a518d2f7e`: numeric secondary-to-subordinate intervals
were treated as hierarchy even when unrelated bridges overlapped, a child
escaped its parent's interval, or a present function had no unique parent
chain to bus zero; and an odd-length DMAR scope path passed validation while
the iterator silently discarded its last byte and could resolve a different
requester. D-370 refused two competing scoped units when their live bridge
paths made the conflict visible, but its bridge callback still read routing
piecemeal, its hierarchy coverage still accepted a numeric interval, and
requester presence and RMRR lookup were separate live observations, so no
single object proved that every requester, bridge range, ancestry decision and
reserved region referred to the same topology.

**Alternatives.** Resolving only conflicts between two DMAR scopes misses one
malicious hierarchy overlapping a catch-all branch. Trusting multifunction
bits can omit a DMA-capable function. Trusting only numeric intervals cannot
distinguish siblings from ancestors. Re-reading configuration independently
for unit selection, requester inventory and RMRR creates time-separated views
with no shared authority. Keeping the complete presence bitmap in a candidate
frame spends more than one page of the boot stack before any VT-d owner moves.

**Consequences.** A numeric range without ancestry falls through to catch-all
and maps no reserved region, and a valid path past the retired eight-hop limit
still outranks another unit's outer hierarchy. Release, debug, shipped and
degraded boots stay green without weakening the candidate stack margin.
Mutations 1,680 through 1,702 each fail their exact named path, topology,
ancestry or production-gate witness.
