# D-371 DMAR authority comes from one complete sealed PCI topology

Status: Accepted (2026-10-09), amends D-358 and D-370; closes PW-581

**Context.** Deep Codex Security scan
`32245094-b8b7-47d1-b6e7-a52554800772` found two remaining ways firmware or
bridge configuration could assign DMA authority to the wrong VT-d unit. First,
numeric secondary-to-subordinate intervals were treated as hierarchy even when
two unrelated bridges overlapped, a child escaped its parent's interval, or a
present function had no unique parent chain to bus zero. Second, a DMAR device
scope with an odd path length passed structural validation; the path iterator
silently discarded its last byte and could resolve a different requester.
These are occurrences `occ_4ba847796226668b8378e266` and
`occ_ecda4a10e95e394a518d2f7e`.

D-370 refused two competing scoped units when their live bridge paths made the
conflict visible, but its bridge callback still read routing piecemeal and its
hierarchy coverage still accepted a numeric interval. Requester presence and
RMRR lookup were separate live observations. That left no single object which
proved that every requester, bridge range, ancestry decision and reserved
region referred to the same topology.

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

**Proof required.** Pure parser tests must cover odd and empty endpoint and
hierarchy paths in DRHD, RMRR and ATSR structures, out-of-range device and
function fields, defensive non-resumption and a valid path past the retired
eight-hop limit which must still outrank another unit's outer hierarchy. Pure
topology tests must cover exact presence and ancestry, duplicate parents,
unrooted functions, class/header contradiction, unreadable bus numbers, every
primary/secondary/subordinate relation, child escape and unrelated overlap.
Exact query tests must distinguish function numbers and bind returned ranges
and ancestry to the sealed tree. DMAR selection and RMRR tests must show that a
numeric range without ancestry falls through to catch-all and maps no reserved
region. A source contract must bind snapshot sealing before enumeration,
static ownership with no local snapshot, the complete 256 by 32 by eight probe,
the early unsealed refusal, and all three requester callbacks plus reserved
regions to the snapshot. Each predicate needs an observed exact red mutation.
Release, debug, shipped and degraded boots must remain green without weakening
the candidate stack margin.

**Evidence.** On 2026-10-09 the focused `acpi-tables` suite passed 80 tests and
one doctest, `pci-config` passed 89 tests and two doctests, every registered
kernel graph compiled, and the production topology contract passed. Mutations
1,680 through 1,702 were each applied independently, failed their exact named
path, topology, ancestry or production-gate witness, and were restored before
the pristine suites passed again. The 1,695-entry registry retained permanent
identities through 1,702 and every source anchor remained applicable. A fresh
post-patch review found the retired eight-hop wrong-unit path and the missing
mutation witnesses, drove the corrections above, and found no remaining
concrete bypass on its final restored-source pass. The ordinary gate then
resolved callers for all 1,972 public API items, passed 725 `xtask` tests with
ten deliberate ignores plus the CLI, ESP, vendor and doctest suites, and
verified 72 registry crates, three artifacts and four runtime inputs. Release
and debug QEMU each passed 276/276 checks; the painted NVMe stack retained
18,136 and 8,912 bytes of headroom respectively. The shipped boot responded
and settled after controller fallback, the no-PCI/no-x2APIC boot kept its
optional fallback diagnostic-only, and the no-DMAR boot refused both storage
candidates before bus mastering and settled.

**Alternatives.** Resolving only conflicts between two DMAR scopes misses one
malicious hierarchy overlapping a catch-all branch. Trusting multifunction
bits can omit a DMA-capable function. Trusting only numeric intervals cannot
distinguish siblings from ancestors. Re-reading configuration independently
for unit selection, requester inventory and RMRR creates time-separated views
with no shared authority. Keeping the complete presence bitmap in a candidate
frame spends more than one page of the boot stack before any VT-d owner moves.
