# D-86 Mapping stops being one-way, because the refusal that made it safe is not the signature

Status: Accepted, 2026-08-24. Amends D-45.

**Decision.** `activate` takes `&mut AddressSpace` and the caller keeps the
space. The kernel holds its own in `kernel/src/mem.rs`, reached through
`with_active_space`. That is a token-bearing acquisition (D-85), with the frame
allocation a new table needs as a leaf underneath.

Mapping into a live space is sound for three checkable reasons:

1. **No live mapping is re-pointed.** D-77 hardened `set` so that a present
   entry accepts only an identical rewrite, anything else being
   `MapError::Conflict`, so nothing the running kernel stands on can move.
2. **No invalidation is owed for a new mapping.** A translation that was not
   present was not cached, so a previously absent page needs no `invlpg`.
   Unmapping is the operation that owes a flush.
3. **One `CR3` owner.** A second `activate` is still refused. That check is
   not the reason `map` is safe.

**Rationale.** D-45 made `map` safe by making it unreachable, taking the space
by value; but the type was never what kept `map` sound, because D-77's entry
check had already closed the hazard. Removing a constraint found already
redundant is the cheapest progress, and it needs the check that made it
redundant to be checkable: this project's strongest argument yet for D-77's
posture that green is a claim to attack. An `ActiveSpace` / `InactiveSpace`
typestate is the obvious alternative, and `010` §4 names it a known failure:
*Theseus's `ActivePageTable`/`InactivePageTable` typestate was simply wrong
once two cores existed.*

**Alternatives.** Keep `activate` consuming and add a separate late-mapping API
(rejected: two APIs for one operation, and the second would need the same
refusal to be sound, so the first buys nothing). The typestate (rejected on
`010` §4's evidence). Leaving the space in `wost` (rejected on D-43: which
ranges exist is policy).

**Consequences.**

- D-45's "mapping is one-way" is retired. D-52's stack cap, D-47's direct-map
  heap and D-49's map-before-claim were all *consequences* of it and are now
  revisable, but none changes here.
- **Unmap does not exist yet.** Until it does, a frame mapped into the live
  space cannot return to the allocator, since freeing it hands back a live
  mapping's page. The check this slice adds leaks one frame on purpose and says
  so. The flush token D-70 rule 9 requires arrives with unmap.
- The check `a mapping added after the switch is live` walks the physical tables
  via the direct map. A `map` that wrote nothing, or wrote into a table `CR3`
  does not point at, fails it. It cannot show the TLB or touch the page:
  `kernel` may not contain a raw write (D-13).
