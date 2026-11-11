# D-111 A program receives exactly one handle at birth, and it is a channel

Status: Accepted, 2026-08-24. Provenance amended by D-151; opaque identity implementation amended by D-248. Amends D-07.

**Decision.** `user::spawn` creates one opaque, typed bootstrap identity before
the thread exists and puts it in a fresh `HandleTable` as the child's only
entry. The table is made and filled inside the `Reserved` window
`spawn_in_space` opens (D-106), so it is never observable holding anything else:
not "we only ever insert once", but "there is no moment at which a second entry
could be seen". D-248 removed the original paired parent endpoint: no current
operation could use it, so creating and orphaning it gave a parent no authority.

The current provenance rule is channel receipt or derivation through a
handle-authorized ring operation, never ambient minting (D-151, D-163). `002`'s
original "channels are the sole delegation mechanism" was false even of
Zircon, which also passes handles through `zx_process_start`; the birth count
is what remains.

**Rationale.** The count matters most. "It has a bootstrap channel" would hold
with a second handle granted beside it. A second grant is the regression this
invariant prevents: one harmless-looking line that turns a capability system
into an ambient one. Self-test `Birth` records the table's live count before
runnability so the check can assert `== 1`.

Quota is 64 handles per process (D-78), which answers kernel-memory exhaustion
at the handle layer. At this decision nothing gave a program a second handle;
D-151's completion reservation and D-163's shared table made that room usable.

**Consequences.** `spawn` returns a `Spawned` carrying the thread slot and, in
self-test builds, the table identity that keys its birth and death evidence. A
check that spawns owns its program, because every ring-3 check counts
crossings and an extra one would be charged to the next check.

Left owed: the program cannot yet *use* its bootstrap identity. The channel
protocol waits behind the Wizzy gate, while current retirement reclaims the
table and its counted identity through the zombie collector.
