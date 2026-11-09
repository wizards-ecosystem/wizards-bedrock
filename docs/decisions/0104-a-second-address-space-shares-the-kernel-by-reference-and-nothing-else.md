# D-104 A second address space shares the kernel by reference and nothing else

Status: Accepted, 2026-08-24.

**Context.** Everything left in slice 7 — `spawn`, `UFrame` (D-44), real
isolation — waits on a second set of page tables. D-86 made a space mappable
while live and D-87 made every mapping carry evidence; what has not existed is
a *second* space to map into.

**Decision.** `AddressSpace::for_user` makes a space whose lower half is empty
and whose higher half is the kernel's, copied entry for entry from the top half
of the installed top-level table.

**Shared by reference, not by contents.** The entries are copied; the tables
beneath them are not. Both spaces walk the same second-level tables, so the
entry path, the thread stacks, the direct map and the heap are reachable from
either without being mapped twice — and a kernel mapping made later appears in
both, because both are looking at the same table.

That last part has a boundary worth naming: it holds only for a mapping that
lands in a top-level slot **that already exists**. One needing a new slot — a
new five-hundred-and-twelve-gigabyte region — would be invisible to spaces made
before it. Nothing does that today, because every window this kernel uses is
placed before the first space is made, and this is the sentence to read when
that stops being true.

**The empty lower half is the isolation.** Two programs' spaces will share the
kernel and nothing else. Not a claim about mechanism: a walk in one space finds
nothing at an address the other has a page at, which is the check.

**Rationale.** The alternative — mapping the kernel into each space
independently — is what a kernel does when it wants the halves to diverge, and
this one does not: there is one kernel and every space needs all of it. Copying
512 entries once per space is cheaper than any alternative and cannot drift,
because there is nothing to keep in step.

Sharing entries also means a user leaf can never make the kernel's half
reachable. `set` upgrades a parent to `USER` only when a user leaf beneath it
needs one, and user leaves are in the lower half; the shared entries stay
supervisor. D-103's narrowing makes that structural rather than incidental —
even a `USER` parent would grant nothing over a supervisor leaf.

**Consequences.**

- One hundred and five named machine checks. The sharing is checked by
  *physical identity*, not by both walks succeeding: two spaces that each
  mapped the kernel separately would both resolve and would not be sharing.
  And the emptiness is checked at an address the live space *does* map — the
  running program's code page — so the difference between the halves is what
  the check sees rather than the absence of everything.
- Breaking it in either direction goes red: sharing nothing, and sharing all
  512 entries.
- `AddressSpace::lookup` is how anything is asked of a space before it is
  installed, which is the only way to check one that is not the CPU's.
- **Nothing installs one yet.** Switching `CR3` on a context switch is the next
  step, and it brings questions this entry does not answer: which space a
  thread belongs to, what `MappedStack::is_installed` means when the kernel's
  stacks are valid in every space, and a full TLB flush on every switch until
  there is a PCID.
