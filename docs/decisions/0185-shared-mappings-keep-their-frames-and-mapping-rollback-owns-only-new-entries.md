# D-185 Shared mappings keep their frames, and mapping rollback owns only new entries

Status: Accepted, 2026-08-27. Amends D-113 and D-115, closes PW-100 and PW-101.

**Decision: shared mappings retain ownership.** `Shared` has one inaccessible
reference-counted allocation. It remains the sole safe data-access handle and
is deliberately not `Clone`; a private keepalive can extend the allocation's
lifetime but exposes no bytes. Every successful `map_shared` stores one such
keepalive in the `AddressSpace`. A successful `AddressSpace::reclaim` drops the
keepalives only after the existing `Released` proof says no task can install
the tables. Dropping an unbound, inactive space also releases them because no
safe root token can reach those mappings. Dropping a bound or active space
leaks the keepalives with the already-leaked tables: loss is preferable to
recycling a frame a CPU may still reach.

**Decision: a mapping transaction has one initial state.** `map_range`
preflights every target entry without allocating or writing. A range that is
entirely absent may be installed and its partial failure may clear everything
written so far. A range that is entirely identical succeeds without writing.
A mixed old/new range is refused before either part changes. This deliberately
narrows idempotence: callers may repeat a whole mapping, but may not use a
larger mapping to absorb an older prefix. No current caller needs the latter,
and refusing it is a checkable transaction boundary rather than a rollback log
whose size depends on the mapping length before the heap exists.

**Rationale.** The D-184 foundation audit found `map_shared` borrowed a
`Shared` and retained nothing, so dropping the owner returned frames the page
tables still named, and `map_range`'s rollback erased an identical entry that
predated the request. A prose outlives obligation cannot make a safe function
sound. The keepalive makes frame lifetime a property of the mapping owner, and
the private non-accessing reference preserves `Shared`'s exclusive safe
accessor. For rollback, preflight keeps the boot-time path allocation-free and
makes the old `undo` argument true: every leaf it clears was absent when this
transaction began.

**Alternatives.** Making `map_shared` unsafe was rejected because the lifetime
is mechanically enforceable. Leaking every shared run forever was safe but
discarded deterministic cleanup. A public clonable `Shared` was rejected
because two clones would expose simultaneous mutable access. Recording every
created leaf in a `Vec` was rejected because the kernel builds its first tables
before the heap exists; a fixed rollback log would replace one defect with an
arbitrary mapping ceiling. Tagging transient entries with a software PTE bit
was rejected as a new architectural state for a property a read-only preflight
can establish.

**Consequences.** An address space grows one empty `Vec`; only shared mappings
allocate capacity, and `try_reserve` turns that allocation failure into
`MapError::OutOfMemory`. One machine check drops the original shared owner and
requires the allocator to withhold its frame until the now-unreachable space
is reclaimed. A second constructs an identical first entry and conflicting
second entry and requires both older entries to survive the refusal. Each
protection has a persistent mutation targeting its exact named check.
