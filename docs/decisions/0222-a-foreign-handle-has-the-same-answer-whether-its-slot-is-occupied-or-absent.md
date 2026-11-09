# D-222 A foreign handle has the same answer whether its slot is occupied or absent

Status: Accepted, 2026-08-29. Amends D-78 and D-95; closes PW-403.

**Context.** D-78 made one global arena safe through a final table-owner
comparison, but gave that comparison its own `NotYours` result while empty and
reused slots returned `Stale`. D-95 made the distinction load-bearing in a
check so deleting the ownership comparison could no longer pass unnoticed.
That proved authority did not cross, but it also preserved the oracle later
named by D-115: given the correct table mixer and an exact slot-generation
candidate, the two errors revealed whether another table still occupied it.

The maximum architectural result leak was one bit per exact candidate:
`NotYours` meant a matching live foreign entry, while `Stale` meant no matching
entry. Repeating probes could inventory candidates but reveal no object or
authority. The shipped ring already collapsed every resolution error to
`STATUS_BAD_HANDLE`; leaving the distinction in the kernel API would let a
future direct Wizzy handle operation accidentally freeze it into a new ABI.

**Decision.** `HandleError::NotYours` is removed. Resolve and close still
compare the full non-repeating table identity, but an owner mismatch returns
`Stale`, exactly as an empty or reused slot does. `Malformed` remains distinct:
it describes the caller's value rather than arena state. The mixer remains a
deterministic accident barrier rather than a secret or an authority check.

This closes the architectural result oracle, not every timing side channel.
Handle lookup is not promised constant-time, and no authorization property
depends on occupancy being secret. The stable interface guarantee is the one
the implementation can prove: live-foreign and absent probes return the same
error and neither changes either table.

**Proof.** The real-arena machine experiment inserts an object in one table,
encodes its exact slot and generation with a second table's mixer, and applies
both resolve and close. It then closes the owner's handle and repeats both
operations against the now-absent candidate. All four observations are
`Stale`; the foreign attempts leave the owner and both counts unchanged.
Deleting the resolve ownership comparison makes the live probe succeed;
deleting the close comparison removes the other table's entry. Each defect
reddens only `a handle reveals no foreign occupancy`.

**Alternatives.** Keeping the distinction because today's ring flattens it was
rejected because the pre-Wizzy foundation exists to settle precisely these
future seams before a caller freezes them. Replacing the global arena with one
allocation per table would remove shared occupancy but discard D-78's bounded
commitment and scaling shape for an error distinction no caller needs. A
secret mixer would require an entropy policy Phase 8 owns and would still not
replace the owner comparison.

**Consequences.** The machine-check count stays 212 because D-95's cross-table
check is strengthened rather than duplicated. Both ownership paths were
broken independently and restored; mutations 232 and 233 retain them. The
full release/debug result is recorded in PW-403's closing row.
