# D-261 Entry-slot identity owns translation eviction

Status: Accepted, 2026-08-31. Amends D-88, D-94 and D-214; closes PW-131.

**Decision.** A private, non-duplicable `EntrySlot` is the one volatile access
boundary for a validated direct-mapped page-table entry. Safe pointer formation
and table-index bounds establish its address; its private read and write
primitives each contain exactly one volatile unsafe operation.

Every clear which can remove a cached mapping disables interrupts, walks the
actual current CR3 to the cleared mapping's level, clears the target slot, and
executes `invlpg` iff the walk reached that identical slot. Mapping undo,
ordinary unmap and device unmap share this rule. Rolling back an empty child
table writes its proved parent link directly: every leaf written through that
link was already cleared and invalidated, or no present leaf ever existed, so
the link carries no TLB debt. Inactive lower mappings rely on the existing
no-PCID, non-global CR3 reload contract; callers still cannot admit `GLOBAL`.

`pushfq`/`pop`, `ltr` and segment selector loads have architectural memory
effects, so their assembly makes no `nomem` promise, and safe pointer formation
opens no unsafe block.

**Rationale.** Synchronous invalidation removed the lifetime hazard from
`Unmapped`, but deciding from page-table-root identity was wrong in both
directions: distinct roots share higher-half tables, so a live translation can
reach the same entry even when the roots differ, while distinct lower-half
tables can hold different entries for one virtual address, so invalidating
because the address matches evicts an unrelated current translation.
Duplicated raw entry reads and writes obscured which identity the decision was
actually comparing.

**Alternatives and evidence.** Root equality was rejected because it misses
shared higher-half slots. Unconditional invalidation was rejected because the
same address in an inactive lower root may denote a different leaf. Deferring
the decision to `Unmapped` was rejected because a safe caller may forget that
frame-return token.

A machine check warms two user roots whose same lower virtual address reaches
distinct physical leaves, removes the inactive victim and observes the current
translation, byte, leaf and invalidation tally unchanged; the active
replacement and shared-higher-half checks prove every required eviction.
Substituting root equality fails only the shared-slot witness, and forcing
invalidation fails only the distinct-slot witness. Both breaks remain retained
beside three exact false-`nomem` reintroductions caught by the unsafe gate.
