# D-329 Device publication and closure prose keep exact source authority

Status: Accepted (2026-09-28), amends D-249, D-265 and D-325; advances PW-010, PW-011 and PW-129

**Context.** D-249 required framebuffer rows to pass through checked volatile
MMIO before their RAM shadow changed, but its retained row-plan and cursor
witnesses did not bind that complete call chain. D-265 called PCI function
authority opaque while its compiler contract rejected raw-address and
duplication routes without separately proving that the pure registry lease and
hardware claim identity stayed private. Separately, D-325's hostile-surface
mutations named rows which the phase close must soon move from `READY` to
`DONE`, and the storage summary copied mutable sweep totals which only a
source-bound qualification record can make authoritative.

**Decision.** The framebuffer contract resolves the exact inherent methods and
requires one checked row plan to feed an exact per-span body: destination,
bounded shadow source, checked MMIO-slice publication and only then shadow
commit. The MMIO slice must delegate its complete iterator to a body containing
exactly one whole-span check followed by pointer derivation and one volatile
write per admitted word. Separate compiler probes must fail specifically with
private-field error `E0451` when they destructure the pure PCI registry lease
or the hardware claim identity.

Closure checks depend on register state rather than the spelling of today's
live rows. One retained defect injects a known-completed row into the
cross-cutting table, and another makes that completed row live without adding
an audit disposition. The storage summary states only the mutation registry's
source-derived count and delegates all sweep outcomes to qualification records;
it cannot preserve caught, survivor or inconclusive totals as current prose.

**Evidence.** Exposing either PCI field made its compiler probe compile and
made the exact authority contract red. Replacing the MMIO delegation with an
unchecked nonvolatile copy and committing the shadow before device publication
each reddened the exact framebuffer contract; mutations 1,392 through 1,395
retain those four defects. The closure mutations independently reported
`PW-542 (DONE)`, missing live `PW-542`, a 1,394-versus-1,395 registry mismatch
and prose claiming authority over sweep verdicts. Byte-for-byte restoration
made both focused prose tests and the device contracts green.

**Alternatives.** A denylist of raw pointer calls was rejected because it can
miss a new bypass while rejecting an unrelated method with the same name.
Event filtering was rejected because ignored statements can shadow a proven
binding or make the matched loop a decoy. Keeping exact `PW-006` or `PW-543`
text as a mutation anchor was rejected because closing the row would rot the
proof. Updating mutable sweep totals after each run was rejected because the
table is not sealed to the commit, tool invocation or complete outcome set.

**Amended by D-341.** Mutations 1,392 through 1,395 above are permanent ids
1,409 through 1,412. Of the rows named in the heading, PW-011 and PW-129 cite
this record; PW-010 does not, and this record's census contribution is the
re-anchored D-325 cross-cutting mutation.

**Amended by D-342.** The rule for the MMIO slice body above, one whole-span
check and one volatile write per admitted word, had no retained break;
mutations 1,451 and 1,452 now make it fail.
