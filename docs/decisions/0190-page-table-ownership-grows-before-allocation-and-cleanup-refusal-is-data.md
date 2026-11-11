# D-190 Page-table ownership grows before allocation, and cleanup refusal is data

Status: Accepted, 2026-08-27. Amends D-117 and closes PW-104.

**Decision.** An `AddressSpace` keeps every non-root table address in a
fallibly grown `Vec<Paddr>`. Before `set` takes and forgets a table frame it
reserves one ownership slot. If that metadata cannot grow, mapping returns
`MapError::OutOfMemory` before a new frame exists; once a frame is taken, its
address is always retained until terminal reclamation. The root remains a
separate field because it must exist before the space can own anything else.

Reclamation now returns `Result<usize, ReclaimError>`. Its errors distinguish
an already reclaimed space, a bound space without release evidence, evidence
for another root, and the space currently in `CR3`. The process collector logs
the exact refusal, carries it in `Reclaimed`, and increments a refusal tally.
The repeated lifecycle proof requires that tally not to move.

**Rationale.** D-117 remembered only thirty-two non-root table frames and
admitted the rest as a `(tables, lost)` leak, and the zombie collector turned
every `reclaim` refusal into `(0, 0)`, so a cleanup failure looked like a
process that owned no tables. Raising the array ceiling only chooses a later
leak. Dynamic ownership makes memory pressure a refusal before ownership changes
rather than an unbounded loss after it. Walking tables recursively at reclaim
time could recover frames without metadata, but would add a second raw
traversal which must distinguish user-owned lower tables from shared kernel
tables. Retaining the addresses when they are minted is smaller and keeps that
distinction out of the unsafe path. Automatic `Drop` reclamation is not an
alternative: a bound space needs task-table release evidence, and only its
construction owner knows which mapped frames must be removed before the tables.

**Consequences.** `a large space returns every table frame` maps one described
page at thirty-four addresses separated by two megabytes. That creates
thirty-six non-root tables plus the root, beyond D-117's ceiling, and the check
requires both the reported count and the physical allocator to return exactly
to baseline. The existing bound-space refusal now also requires an unchanged
frame tally, and forty program lifecycles require no process table-reclaim
refusal. Limiting the reclaim drain to thirty-two entries reddened only the new
check; that defect is mutation 95. No unsafe operation was added.
