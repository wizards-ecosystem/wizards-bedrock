# D-86 Mapping stops being one-way, because the refusal that made it safe is not the signature

Status: Accepted, 2026-08-24. Amends D-45.

**Context.** D-45 made `AddressSpace::map` safe by making it unreachable:
`activate` took the space **by value**, so once the tables were live nothing
held anything to map into. That decision shaped three phases. Thread stacks are
committed at table-build time (D-52) and their count is a real cap; the I/O APIC
window is mapped before the switch and claimed after (D-49); the heap allocates
out of the direct map because it cannot map (D-47). `009` and `007` both record
"how the address space grows" as a decision Phase 5 must take deliberately, and
every slice from 3 onward is blocked on it — a stack cannot arrive with its
thread, and a memory object cannot arrive with its process.

**The finding.** The property that keeps `map` sound was never the signature.
D-77 hardened `set` so that **a present entry accepts only an identical
rewrite**: anything else is `MapError::Conflict`, because "it would re-point a
mapping some handle is evidence of". That refusal is what stops a live mapping
being changed under the running kernel, and it holds whether or not the tables
are active. Taking the space by value was protecting against a hazard the entry
check had already closed.

**Decision.** `activate` takes `&mut AddressSpace` and the caller keeps the
space. The kernel holds its own in `kernel/src/mem.rs`, reached through
`with_active_space`, a token-bearing acquisition (D-85) with the frame
allocation a new table needs as a leaf underneath.

The soundness argument for mapping into a live space is stated in three parts,
each checkable rather than promised:

1. **No live mapping is re-pointed.** `set` refuses any change to a present
   entry, so nothing the running kernel stands on can move.
2. **No invalidation is owed for a new mapping.** A translation that was not
   present was not cached, so a previously absent page needs no `invlpg`.
   Unmapping is the operation that owes a flush, and it does not exist yet.
3. **One `CR3` owner.** A second `activate` is still refused, and the check
   that says so remains — with its rationale corrected, because it used to
   claim it was the reason `map` was safe.

**Rationale.** Removing a constraint by discovering it was already redundant is
the cheapest kind of progress available, and it is only available if the check
that made it redundant was written to be checkable. D-77 wrote that check for a
different reason — an adversarial pass looking for safe callers that could reach
UB — and this phase collected the interest. It is the strongest argument in this
project so far for D-77's posture that green is a claim to attack.

The alternative shapes were both worse. An `ActiveSpace` / `InactiveSpace`
typestate is the obvious Rust answer and `010` §4 names it as a known failure:
*Theseus's `ActivePageTable`/`InactivePageTable` typestate was simply wrong once
two cores existed.* A `wost`-held global reachable through a closure would put a
lock where the kernel's own policy belongs, against D-43.

**Alternatives.** Keep `activate` consuming and add a separate late-mapping API
(rejected: two APIs for one operation, and the second would need the same
refusal to be sound, so the first buys nothing); the typestate (rejected on
`010` §4's evidence); leaving the space in `wost` (rejected on D-43 — which
ranges exist is policy).

**Consequences.**

- D-45's "mapping is one-way" is retired. D-52's stack cap, D-47's direct-map
  heap and D-49's map-before-claim were all *consequences* of it and are now
  revisable — but none of them changes here. Each moves when it has a reason
  of its own, which is slice 3's stacks and slice 7's memory objects.
- **Unmap still does not exist**, and until it does a frame mapped into the
  live space cannot be returned to the allocator: freeing it would hand a live
  mapping's page back. The check this slice adds leaks one frame deliberately
  and says so. The flush token D-70 rule 9 requires arrives with unmap.
- The check `a mapping added after the switch is live` walks the physical
  tables through the direct map, so a `map` that wrote nothing or wrote into a
  table `CR3` does not point at fails it. What it cannot show is the TLB, and
  what it cannot do is touch the page: `kernel` may not contain a raw write
  (D-13), and the safe type for reaching a mapping is the memory-object mapping
  in slice 7. The check grows its second half there.
- Eighty-one named machine checks. The break is to drop the space again, as
  D-45 left it, and the check goes red.
