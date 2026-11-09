# D-401 Interrupt-boundary source contracts prove duplicate guards and live predicates

Status: Accepted, 2026-10-13. Amends D-372 and D-395; advances PW-001.

**Context.** Permanent mutations 1,719 and 1,724 survived the host contract
`pci_interrupts_are_default_deny_before_bus_mastering_or_sti`. Mutation 1,719
disabled the DMAR interrupt-remapping and x2APIC-opt-out refusal in the sealed
topology lookup, but the contract found the identical earlier refusal in
`establish_fabric_boundary` and credited it to both locations. Mutation 1,724
made the PCI-topology keyboard rollback condition permanently false, but the
contract still found the topology call and the later rollback and return tokens
in source order. The production source retains both firmware refusals and the
live PCI routing veto; the survivors expose token-presence proof gaps rather
than an interrupt or DMA escape.

**Decision.** The source contract requires exactly two copies of the canonical
DMAR interrupt-remapping refusal: one before early fabric ownership and one at
sealed-topology unit lookup. It also requires the exact live
`!matches!(topology_presence(), Empty)` branch inside `route_keyboard`, in
addition to the existing order proof that rollback and return precede the only
commit. A duplicate guard can no longer lend credit to a removed sibling, and
a dead Boolean prefix can no longer satisfy the routing boundary.

**Proof required.** The restored host contract passes. Applying mutation 1,719
alone must fail because only one firmware refusal remains, and mutation 1,724
alone must fail because the exact live topology predicate is absent. Each
source must restore byte for byte; the registry application checks, ordinary
gate and next complete clean sweep must retain both results.

**Alternatives.** Removing the later firmware check as redundant was rejected
because unit lookup is a separately audited authority boundary and the source
contract intentionally keeps its local preconditions explicit. Adding another
runtime boot was rejected because the qualified QEMU firmware advertises both
features and cannot exercise their absence without changing the platform
contract. Searching only for the condition's inner tokens was rejected because
the survived `false &&` form preserves every such token while deleting the
decision.
