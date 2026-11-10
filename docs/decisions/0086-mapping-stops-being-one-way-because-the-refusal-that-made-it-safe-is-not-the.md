# D-86 Mapping stops being one-way, because the refusal that made it safe is not the signature

Status: Accepted, 2026-08-24. Amends D-45.

**Context.** D-45 made `AddressSpace::map` safe by making it unreachable:
`activate` took the space **by value**, so once the tables were live nothing
held anything to map into. That decision shaped three phases. Thread stacks are
committed at table-build time (D-52) and their count is a hard cap. The I/O APIC
window is mapped before the switch and claimed after (D-49). The heap allocates
out of the direct map because it cannot map (D-47). `009` and `007` record "how
the address space grows" as a choice Phase 5 must make on purpose. Every slice
from 3 onward is blocked on it: a stack cannot arrive with its thread, nor a
memory object with its process.

**The finding.** The function's type was never what kept `map` sound. D-77
hardened `set` so that **a present entry accepts only an identical rewrite**.
Anything else is `MapError::Conflict`, because "it would re-point a mapping some
handle is evidence of". That refusal stops a live mapping changing under the
running kernel, active tables or not. Taking the space by value guarded a hazard
the entry check had already closed.

**Decision.** `activate` takes `&mut AddressSpace` and the caller keeps the
space. The kernel holds its own in `kernel/src/mem.rs`, reached through
`with_active_space`. That is a token-bearing acquisition (D-85), with the frame
allocation a new table needs as a leaf underneath.

Mapping into a live space is sound for three checkable reasons:

1. **No live mapping is re-pointed.** `set` refuses any change to a present
   entry, so nothing the running kernel stands on can move.
2. **No invalidation is owed for a new mapping.** A translation that was not
   present was not cached, so a previously absent page needs no `invlpg`.
   Unmapping is the operation that owes a flush, and it does not exist yet.
3. **One `CR3` owner.** A second `activate` is still refused, and that check
   remains. Its rationale is corrected, because it used to claim it was the
   reason `map` was safe.

**Rationale.** Removing a constraint found already redundant is the cheapest
progress, and it needs the check that made it redundant to be checkable. D-77
wrote that check for another reason, an adversarial pass looking for safe
callers that could reach UB, and this phase collected the interest. It is this
project's strongest argument yet for D-77's posture that green is a claim to
attack.

Both alternative shapes were worse. An `ActiveSpace` / `InactiveSpace` typestate
is the obvious Rust answer. `010` §4 names it a known failure: *Theseus's
`ActivePageTable`/`InactivePageTable` typestate was simply wrong once two cores
existed.* A `wost`-held global reachable through a closure would put a lock
where the kernel's own policy belongs, against D-43.

**Alternatives.** Keep `activate` consuming and add a separate late-mapping API
(rejected: two APIs for one operation, and the second needs the same refusal to
be sound). The typestate (rejected on `010` §4's evidence). Leaving the space in
`wost` (rejected on D-43: which ranges exist is policy).

**Consequences.**

- D-45's "mapping is one-way" is retired. D-52's stack cap, D-47's direct-map
  heap and D-49's map-before-claim were all *consequences* of it and are now
  revisable, but none changes here. Each moves for a reason of its own: slice
  3's stacks and slice 7's memory objects.
- **Unmap still does not exist.** Until it does, a frame mapped into the live
  space cannot return to the allocator, since freeing it hands back a live
  mapping's page. The check this slice adds leaks one frame on purpose and says
  so. The flush token D-70 rule 9 requires arrives with unmap.
- The check `a mapping added after the switch is live` walks the physical tables
  via the direct map. A `map` that wrote nothing, or wrote into a table `CR3`
  does not point at, fails it. It cannot show the TLB or touch the page:
  `kernel` may not contain a raw write (D-13), and the safe type for reaching a
  mapping is slice 7's memory-object mapping. The check grows its second half
  there.
- Eighty-one named machine checks. The break drops the space again, as D-45 left
  it, and the check goes red.
