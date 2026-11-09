# D-341 Mutation citations name permanent identities

Status: Accepted (2026-09-28), amends D-244, D-325, D-326, D-327, D-328 and D-329; closes PW-550

**Context.** Documents cited mutations by the runner's array position, and
positions were not stable. Since the 2026-09-26 registry, 19 entries were
placed at the top of the array (three on 2026-09-27, sixteen in `0a96973`) and
one was inserted mid-array (`3a50327`). Every earlier citation therefore moved
by a different amount: +3, +16, +17, +19 or +20, and older rows by hundreds
after years of neighbouring inserts. Two numbers were wrong on the day they were
written: D-326's "1,389" named what was then position 1, and PW-542's closing
"1,301-1,312" copied D-288's range after it had already moved. Nothing checked a
citation against the registry, so a cited number silently came to name an
unrelated defect.

**Decision.** Every mutation carries a permanent `id`. The existing entries
take their positions at `cb2a313`, so the latest citations stay valid, and a
new entry takes `NEXT_MUTATION_ID` wherever it is placed. A deleted entry's id
moves to `RETIRED_MUTATION_IDS` and is never issued again; one host test
requires the registered and retired ids to be exactly `1..NEXT_MUTATION_ID`,
each once. The runner prints `#id` beside every progress, restore, survivor and
inconclusive line; the registry fingerprint hashes every id; and the sequenced
mutation report, now `wizard-mutation-report-v2`, names each outcome's id. A
report keyed by an older registry fingerprint is never read by a newer sweep,
so no compatibility path is needed.

The living register, census, storage record, README and AGENTS cite ids. A
prose check parses every `mutation N`, list and range citation in them and
requires each id to be registered, or retired where the same line says so.
Decision records keep the numbers their evidence was recorded against; the
appendix below maps the three recent records whose numbers no longer name the
same entry. The register's citations were re-derived by content, matching each
row's defect and witness to an entry's `breaks` text and named check rather than
applying an offset.

Three related rules land with it. D-325's census check now refuses a second
`OPEN` segment in one proof cell, an `OPEN` label outside the proof column and
a cross-cutting row it cannot read, where each previously escaped
classification. The register's sweep guide states its exact QEMU boot count,
checked against the machine-witnessed entries. The storage record's count
mutation (id 21) changes the count's leading digit instead of quoting the whole
number, so appending an entry no longer rewrites that mutation.

**Evidence.** On 2026-09-28 all 1,413 existing entries took ids equal to their
`cb2a313` positions, and `every_mutation_still_applies`, the identity
accounting test, the report round trip with non-positional ids, the citation
parser, the living-citation check, the hardened census check and the boot-count
check passed. Each new check was then broken once: a duplicated id failed with
`#17 names both`, an unissued id failed as outside the issued identities, a
report without ids failed its round trip, a doubled segment and an `OPEN` label
in the consumer column each failed the one-segment rule, an unbackticked
cross-cutting row failed as unreadable, a citation of an unissued id failed the
living-citation check and an understated boot count failed the guide check.
Those defects are retained as mutations 1,414 through 1,421. All 51 register
citations were matched by content: 31 were corrected, among them PW-029's 553
(now 569), PW-108's 236 (now 497), PW-130's 1219 (now 839), PW-542's range
(now 1,323 through 1,334) and PW-548's 1,368 (now 1,388); the 20 written after
`0a96973` already named the right entries.

**Alternatives.** Moving the prepended entries back to the end was rejected: it
would repair citations written before `0a96973` and break every one written
after it. An append-only guard over positions was rejected because positions
would still carry meaning while nothing verified a citation. Citing mutations
by their `breaks` text was rejected because that text is long and is itself
corrected over time. Content hashes were rejected for the same reason.

**Appendix: recent decision citations that no longer match.** D-326's
mutation 1,389 is id 17. D-327's mutations 1,390 and 1,391 are ids 1,407 and
1,408. D-329's mutations 1,392 through 1,395 are ids 1,409 through 1,412. The
numbers cited by D-322 through D-340 not listed here already equal their ids.
