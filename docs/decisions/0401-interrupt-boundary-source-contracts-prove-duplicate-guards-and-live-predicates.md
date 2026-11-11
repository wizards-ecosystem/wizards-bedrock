# D-401 Interrupt-boundary source contracts prove duplicate guards and live predicates

Status: Accepted, 2026-10-13. Amends D-372 and D-395; advances PW-001.

**Decision.** The source contract
`pci_interrupts_are_default_deny_before_bus_mastering_or_sti` requires exactly
two copies of the canonical DMAR interrupt-remapping refusal: one before early
fabric ownership and one at sealed-topology unit lookup. It also requires the
exact live `!matches!(topology_presence(), Empty)` branch inside
`route_keyboard`, in addition to the existing order proof that rollback and
return precede the only commit. A duplicate guard can no longer lend credit to
a removed sibling, and a dead Boolean prefix can no longer satisfy the routing
boundary. Mutation 1,719 alone fails because only one firmware refusal remains,
and mutation 1,724 alone fails because the exact live topology predicate is
absent.

**Rationale.** Mutations 1,719 and 1,724 had survived: the contract credited
the earlier `establish_fabric_boundary` refusal to both locations, and a
`false &&` rollback condition kept every token it searched for. Production
retained both refusals and the live routing veto, so these were proof gaps, not
an interrupt or DMA escape. Unit lookup is a separately audited authority
boundary whose local preconditions stay explicit, and the qualified QEMU
firmware advertises both features, so a runtime boot cannot exercise their
absence without changing the platform contract.
