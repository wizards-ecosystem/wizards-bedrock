# D-245 Cleanup authority survives every refusal, and partial construction is fail-atomic

Status: Accepted, 2026-08-30. Amends D-106, D-117, D-190, D-191, D-206 and D-243; closes PW-527.

**Decision.** Cleanup state is complete, typed and retryable. The fixed process
ledger has one entry for every admitted image segment plus the stack, and
overflow is an error rather than a dropped record. Loader and process rollback
return the refusal and exact page progress. A zombie retains either its `Bound`
or `Released` authority, its remaining mappings and its returned-page tally
until the next attempt. Page-table reclaim refuses while any owned leaf remains
reachable and returns the release token on refusal.

`AddressSpace::Drop` remains non-reclaiming: drop has neither the fallible API
nor, for an installed space, the linear release evidence needed to reclaim
tables safely. Every owning production path therefore calls `unmap` and
`reclaim` explicitly. Stack teardown returns a linear progress token, so a
partial refusal resumes below already-unmapped slots rather than failing on
them. Range mapping snapshots its table ownership, clears every written leaf
and detaches newly linked empty tables in reverse order; a contradictory
rollback walk fails stopped before a still-addressable run can be freed.
Kernel-space construction applies the same transaction. `Frame` and `Segment`
drop release the allocator lock and then fail stopped if their live handle is
absent from the ledger.

A zombie table records both stored and currently-collecting owners. Scheduler
selection excludes either state, and process commit checks the slot again
before publishing launch records. Commit refusal returns the still-bound
`PreparedProcess` with its `Bound` token; retirement first yields `Released`
authority, which rejoins construction before drop. Task retirement is completed
before the scheduler slot is released: one injected first refusal is retried,
and a second refusal is an internal contradiction.

Ring 3 also requires isolated extended state before its first allocation.
Absent or contradictory XSAVE state now produces `NoExtendedState`; boot may
continue, but user launch stays closed. Kernel stacks remain guarded and are
32 KiB, because the linear construction path exceeded the former 16 KiB debug
stack; a measured publication point on the restored build retained 11,216 bytes
above the guard. No guard was weakened.

**Rationale.** Re-auditing cleanup as hostile code found one defect at four
ownership layers: the ledger dropped its final entry for a maximum-size image
with a stack; loader rollback, construction drop and zombie collection
discarded `unmap` or release refusals, the last consuming the only `Bound`
token that could authorize a retry; boot paging dropped an unreclaimed root and
kept a partial stack arena; and task construction ignored a retirement refusal
while a free scheduler slot could still carry an older zombie, so a later
orphan could then replace the retained owner. Because `AddressSpace::Drop`
cannot walk or free live mappings, the discarded results were safe only by
leaking every remaining owner. User launch also remained open when XSAVE
isolation was unavailable, and `Frame` and `Segment` drop ignored
allocator-ledger refusal even though D-206 classifies such internal ownership
contradiction as fatal.

**Alternatives.** Treating cleanup refusal as a harmless leak was rejected:
it loses retry authority and turns bounded pressure into permanent exhaustion.
Reclaiming implicitly from `Drop` was rejected because drop cannot return a
typed refusal or manufacture installation authority. Reusing a scheduler slot
and relying only on the final process-commit check was rejected because it
makes ordinary admission perform and then unwind avoidable construction.
Growing stacks again after one unreproduced fault was rejected after the clean
build's measured headroom and interrupt-frame disassembly did not support it.

**Evidence and consequences.** Machine checks prove the maximum image-and-stack
ledger, refusal after one successful unmap, resumable full-prefix and guard
rollback, failed boot-space and range-map table reclamation, commit-authority
reunion, retirement retry, zombie non-aliasing/retry and the no-XSAVE gate.
Host ELF tests prove power-of-two `p_align` and file/address congruence; the
machine loader additionally refuses non-page placement before taking a frame.
The integrated release QEMU suite passed 218/218. Exact retained breaks cover progress-token loss, forgotten table rollback,
rollback-walk refusal, commit-owner loss, loader alignment and activation
privilege/cache drift. The table transaction remains inside `wost`'s reviewed
unsafe boundary.
