# D-261 Entry-slot identity owns translation eviction

Status: Accepted (2026-08-31), amends D-88, D-94 and D-214; closes PW-131

**Context.** Synchronous invalidation removed the lifetime hazard from
`Unmapped`, but deciding from page-table-root identity was still wrong in both
directions. Distinct roots share higher-half tables, so a live translation can
reach the same entry even when the roots differ. Distinct lower-half tables can
instead contain different entries for the same virtual address, so invalidating
solely because the address matches evicts an unrelated current translation.
Duplicated raw entry reads and writes obscured which identity the decision was
actually comparing.

The unsafe inventory audit also found three assembly contracts which claimed
`nomem` despite architectural memory effects: `pushfq`/`pop`, `ltr` and segment
selector loads. Several raw-pointer additions needed no unsafe operation at
all. Those claims made compiler constraints, not just prose, part of this
correction.

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

The corrected assembly omits false `nomem` promises, safe pointer formation no
longer opens unsafe blocks, and the D-261 inventory was 356 unsafe lines, 146
blocks and 37 declarations under the unchanged 380-line budget. The later
explicit-dispatch cleanup changed no unsafe operation or declaration; rustfmt
expanded one existing heap write by two lines, making the current reviewed
inventory 358/146/37.

**Alternatives and evidence.** Root equality was rejected because it misses
shared higher-half slots. Unconditional invalidation was rejected because the
same address in an inactive lower root may denote a different leaf. Deferring
the decision to `Unmapped` was rejected because a safe caller may forget that
frame-return token.

Release QEMU passed 252/252. It warmed two user roots whose same lower virtual
address reached distinct physical leaves, removed the inactive victim, and
observed the current translation, byte, leaf and self-test invalidation tally
unchanged; the existing active replacement and shared-higher-half checks still
proved every required eviction. Substituting root equality failed only the
shared-slot witness, and forcing invalidation failed only the distinct-slot
witness. Both breaks remain retained beside three exact false-`nomem`
reintroductions caught by the unsafe fingerprint gate. The budget and one
reviewed fingerprint were each made wrong by hand, failed their exact host
checks, and were restored.
