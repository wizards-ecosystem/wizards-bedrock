# D-111 A program receives exactly one handle at birth, and it is a channel

Status: Accepted, 2026-08-24. Provenance amended by D-151; opaque identity implementation amended by D-248. Amends D-07.

**Context.** `002`'s pillar 1 was amended once already, because its original
wording — "channels are the sole delegation mechanism" — was false even of
Zircon, which also passes handles through `zx_process_start`. The replacement
joined a load-bearing birth count to a channel-only provenance claim. The
birth count remains; D-151 and D-163 later made handle-authorized completions
a second provenance path. Until D-110 there were no channels to give one.

**Decision.** `user::spawn` creates one opaque, typed bootstrap identity before
the thread exists and puts it in a fresh `HandleTable` as the child's only
entry. The table is made and filled inside the `Reserved` window
`spawn_in_space` opens (D-106), so it is never observable holding anything else
— not "we only ever insert once", but "there is no moment at which a second
entry could be seen". D-248 removed the original paired parent endpoint; no
current operation could use it, so constructing and immediately orphaning it
was not authority a parent could exercise.

**Rationale.** The count is the load-bearing half. "It has a bootstrap channel"
would hold with a second handle granted beside it, and a second grant is
exactly the regression this invariant exists to prevent — it is one line, it
looks harmless, and it quietly turns a capability system into an ambient one.
Self-test `Birth` records the table's live count before runnability so the check
can assert `== 1`; installing any second object turns it red.

Quota is 64 handles per process (D-78), which is the answer to kernel-memory
exhaustion at the handle layer. At this decision nothing gave a program a
second handle; D-151's completion reservation and D-163's shared table made
that room usable.

**Consequences.** `spawn` returns a `Spawned` carrying the thread slot and, in
self-test builds, the exact table identity that keys its birth and death
evidence. The check spawns its own program rather than reusing a launcher,
because every ring-3 check counts crossings and an extra one from here would
be attributed to whichever check looked next; that cost two red checks before
it was noticed, and the fix is that a check which spawns owns its program.

`002`'s Zircon correction was already made when the pillar was reworded; what
this decision adds is the code that makes the birth count true of *this*
kernel. The current provenance rule is channel receipt or derivation through
a handle-authorized ring operation, never ambient minting (D-151, D-163).

Left owed: the program cannot yet *use* its bootstrap identity. The channel
protocol waits behind the Wizzy gate, while current retirement reclaims the
table and its counted identity through the zombie collector.
