# D-104 A second address space shares the kernel by reference and nothing else

Status: Accepted, 2026-08-24.

**Decision.** `AddressSpace::for_user` makes a space whose lower half is empty
and whose higher half is the kernel's, copied entry for entry from the top half
of the installed top-level table.

**Shared by reference, not by contents.** The entries are copied; the tables
beneath them are not. Both spaces walk the same second-level tables, so a kernel
mapping made later appears in both.

That holds only for a mapping in a top-level slot **that already exists**. One
needing a new slot, a new five-hundred-and-twelve-gigabyte region, would be
invisible to spaces made before it. Nothing does that today: every window this
kernel uses is placed before the first space is made. Read this sentence when
that stops being true.

**The empty lower half is the isolation.** Two programs' spaces share the
kernel and nothing else.

**Rationale.** There is one kernel and every space needs all of it, so copying
512 entries once per space is the cheapest option and cannot drift. A user leaf
can never make the kernel's half reachable: `set` upgrades a parent to `USER`
only for a user leaf beneath it, user leaves are in the lower half, and D-103's
narrowing means even a `USER` parent grants nothing over a supervisor leaf.

**Consequences.**

- The sharing is checked by *physical identity*, not by both walks succeeding,
  since two spaces that each mapped the kernel separately would both resolve.
  The emptiness is checked at an address the live space *does* map.
- `AddressSpace::lookup` asks a space questions before it is installed, the only
  way to check one that is not the CPU's.
- Every switch needs a full TLB flush until there is a PCID.
