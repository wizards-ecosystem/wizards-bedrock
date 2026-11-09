# D-263 Path authority is retained until publication or destruction commits

Status: Accepted (2026-08-31), amends D-244 and D-245; advances PW-015, PW-016, PW-017, PW-020, PW-021, PW-023 and PW-024

**Context.** Host tools repeatedly validated one pathname and then truncated,
replaced or recursively removed another object reached through the same text.
Symlinks, hardlinks and redirected ancestors turned an ordinary refusal into an
outside write; unlink-before-rename also destroyed the last complete authority
before the replacement was known good.

**Decision.** A recursive deletion or authoritative publication target must be
the exact allowlisted real child of its canonical repository root, with real
non-link ancestors checked immediately before the operation. New authoritative
bytes are built through `create_new` in that parent, synchronized and
revalidated before replacement. When a complete object is being replaced,
publication keeps it as rollback authority until the new name and bytes
validate. Recursive cleanup is narrower: only the exact VMware root may enter
`remove_dir_all`, and a redirected final component or ancestor is a refusal.

Portable pathname preflight does not defeat a concurrent replacement by a
local actor with the same filesystem authority. Directory-handle and no-follow
operations would be needed for that stronger claim; this repository records the
race instead of pretending canonicalization removes it.

**Current evidence.** Host fixtures substitute links at SPDX, QEMU storage,
boot-image, VMware-generation, API-workspace and mutation-workspace boundaries,
and preserve outside sentinels. Rollback fixtures preserve prior complete
bytes. Exact-root, ancestor, construction-failure and rollback mutations
protect every named publication door; current integrated boots and generation
runs remain in their register rows.

**Current disposition (2026-08-31).** PW-015, PW-016, PW-017, PW-020, PW-021,
PW-023 and PW-024 are `DONE`. Their path, publication and rollback mechanisms
have exact host and mutation witnesses; integrated execution or generation
evidence remains with the separate register rows that own it.
