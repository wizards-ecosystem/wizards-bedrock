# D-283 Shared allocator aliases keep one write-back cache policy

Status: Accepted, 2026-09-05. Amends D-185 and D-269; advances PW-128.

**Context.** A shared allocation remains reachable through the kernel's
write-back direct map while ring 3 reaches a second mapping. The safe shared
mapping doors nevertheless accepted `NO_CACHE` and `WRITE_THROUGH`, allowing
one physical run to have incompatible memory types at the same time. Bounds,
ownership and publication ordering do not make that alias safe.

**Decision.** `map_shared` and `map_shared_ring` accept only the allocation's
write-back policy. Either cache-policy flag is refused before page-table
preflight, keepalive reservation or mapping state can change. Device memory
continues through its separate admitted, uncached mapping path; the shared
allocator is not a generic physical-alias constructor.

**Alternatives and proof obligation.** Changing the direct map per allocation
was rejected because it would mutate a global kernel alias and require a
separate shootdown and restoration protocol. Treating one of the two cache
flags as harmless was rejected because the conflict is between aliases, not
the spelling of one entry. The shared-mapping machine witness covers each flag
and their combination for both shared types, requires no installed leaf or
retained owner on refusal, and has a registered single-flag bypass mutation.
Its restored release, debug and VMware executions remain part of PW-128.

**Amended by D-349.** The same write-back rule now covers every RAM mapping,
not only the shared doors: `map_owned`, `map_user` and `map_boot` refuse a
cache type the range's direct map does not already have.
