# D-341 Mutation citations name permanent identities

Status: Accepted, 2026-09-28. Amends D-244, D-325, D-326, D-327, D-328 and D-329; closes PW-550.

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

**Rationale.** Documents cited mutations by the runner's array position, which
moved whenever an entry was placed ahead of it (sixteen entries were prepended
in `0a96973`), and nothing checked a citation
against the registry, so a cited number silently came to name an unrelated
defect; D-326's "1,389" and PW-542's closing "1,301-1,312" were already wrong
on the day they were written. Mutations 1,414 through 1,421 retain one break of
each new check.

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
