# D-419 Generated system-skill identity uses the verifier's ordinal path order

Status: Accepted, 2026-10-17. Implemented with qualification OPEN; amends D-413 and D-417; advances PW-026.

**Decision.** The one allowed system-skills identity remains exactly 71 entries
and 314,963 bytes, but its digest is the production-canonical ordinal value
`54c2abe5f6017b67270f4bbb993b3b1e3a808da31575a1d24174cb4b2edd99c8`.
The legacy culture-sorted digest is refused rather than admitted as an
alternative. Both inventories in the race check use one shared ordinal sorting
function. Future pin refreshes must obtain the identity through the verifier's
measurement implementation or prove byte-for-byte equivalence to its ordering;
an independently reimplemented host sort is not authority.

A mixed-case sibling fixture fixes the ordinal fold independently of host
culture, and a second fixture admits the one production digest and rejects
D-417's legacy value. Mutations 2,093 through 2,095 respectively replace
ordinal ordering with case-insensitive ordering, restore the legacy pin and
admit both digests; each names one of those exact witnesses.

**Qualification boundary.** This correction changes the source after the
successful model and advisory records on `71148f1`, so every exceptional
qualification must be repeated on one new clean frozen commit before PW-026 or
release can close.

**Rationale.** D-417's PowerShell probe sorted paths with culture-aware
`Sort-Object relative` and reported
`1715d9d4d5d41e12e37d33e46d44d970ca64f298846430a20c6af04c8f1caf7c`, while
production Rust orders the same normalized paths with ordinal, case-sensitive
`String::cmp`, which places uppercase `LICENSE.txt` before lowercase `agents`;
no executable, plugin, configuration or skill byte had changed. Rotating the
hash unexplained would hide a provenance defect, only the production
verifier's ordering defines authority, and excluding mixed-case files or
normalizing case would alter names that are part of the exact executable
instruction tree and that Windows preserves.
