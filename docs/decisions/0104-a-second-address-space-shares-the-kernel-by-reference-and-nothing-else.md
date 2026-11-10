# D-104 A second address space shares the kernel by reference and nothing else

Status: Accepted, 2026-08-24.

**Context.** Everything left in slice 7 (`spawn`, `UFrame` (D-44), isolation)
waits on a second set of page tables. D-86 made a space mappable while live and
D-87 made every mapping carry evidence. What did not exist is a *second* space
to map into.

**Decision.** `AddressSpace::for_user` makes a space whose lower half is empty
and whose higher half is the kernel's, copied entry for entry from the top half
of the installed top-level table.

**Shared by reference, not by contents.** The entries are copied; the tables
beneath them are not. Both spaces walk the same second-level tables, so the
entry path, thread stacks, direct map and heap are reachable from either without
being mapped twice. A kernel mapping made later appears in both, because both
look at the same table.

That holds only for a mapping in a top-level slot **that already exists**. One
needing a new slot, a new five-hundred-and-twelve-gigabyte region, would be
invisible to spaces made before it. Nothing does that today: every window this
kernel uses is placed before the first space is made. Read this sentence when
that stops being true.

**The empty lower half is the isolation.** Two programs' spaces will share the
kernel and nothing else. The check is behavioural: a walk in one space finds
nothing at an address where the other has a page.

**Rationale.** Mapping the kernel into each space independently suits a kernel
that wants the halves to diverge. This one does not: there is one kernel and
every space needs all of it. Copying 512 entries once per space is the cheapest
option and cannot drift, since nothing needs keeping in step.

Sharing entries also means a user leaf can never make the kernel's half
reachable. `set` upgrades a parent to `USER` only when a user leaf beneath it
needs one, and user leaves are in the lower half; the shared entries stay
supervisor. D-103's narrowing makes that structural: even a `USER` parent would
grant nothing over a supervisor leaf.

**Consequences.**

- One hundred and five named machine checks. The sharing is checked by *physical
  identity*, not by both walks succeeding. Two spaces that each mapped the
  kernel separately would both resolve without sharing. The emptiness is checked
  at an address the live space *does* map, the running program's code page, so
  the check sees the difference between the halves.
- Breaking it in either direction goes red: sharing nothing, and sharing all
  512 entries.
- `AddressSpace::lookup` asks a space questions before it is installed, the only
  way to check one that is not the CPU's.
- **Nothing installs one yet.** Switching `CR3` on a context switch is next, and
  it raises questions this entry leaves open. Which space does a thread belong
  to? What does `MappedStack::is_installed` mean when the kernel's stacks are
  valid in every space? And every switch needs a full TLB flush until there is a
  PCID.
