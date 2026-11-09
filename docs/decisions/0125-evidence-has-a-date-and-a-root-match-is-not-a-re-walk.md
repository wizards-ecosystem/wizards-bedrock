# D-125 Evidence has a date, and a root match is not a re-walk

Status: Accepted, 2026-08-24.

**Context.** `MappedStack` is the value `map_stack` returns: a stack top, plus
the root of the space that mapped it. It is not `Clone`, so exactly one caller
holds it, and `segment::set_double_fault_stack` takes it as the proof it needs
before writing the CPU's `IST1` entry — the stack the most privileged handler
on the machine will stand on.

`is_installed` was `is_active() && active_root() == self.root`. That answers
*which space the CPU is walking*, and it was written to answer that: addresses
collide across spaces, so the root is the only thing that tells two stacks
apart. What it does not answer is whether the stack is **still there**.

`MappedStack` is evidence dated at construction. Nothing consumes it on unmap,
nothing invalidates it, and the root goes on matching for the life of the
space. So the value outlives the mapping, and a caller holding one can point
`IST1` at an address the tables no longer translate. A double fault then takes
a second fault on its own stack, which is a triple fault: the machine reboots
with nothing on the wire.

D-94 found this shape once already — `map_owned` taking its own doc at its
word — and left this instance on its open list with the honest note that
nothing in the tree unmaps a stack. That is still true. It is not an argument:
the same sentence was true of the user-copy window until D-120 built the check
that reaches it, and true of `reclaim` until D-123 found four callers that
could.

**Options.**

1. **Consume the `MappedStack` on unmap.** The strongest: the type system would
   make a stale one unconstructible. It needs `unmap` to know which of its
   pages were a stack, which it does not, and would put a stack-shaped special
   case into the general path.
2. **A generation on the space, bumped by every unmap.** Cheap to check and
   coarse: any unmap anywhere invalidates every stack in the space, so the
   double-fault stack would stop being installed because a thread exited.
3. **Re-walk the mapping in `is_installed`.** Ask the tables the question the
   caller is actually asking.

**Decision.** Option 3. `is_installed` keeps the root comparison and adds a
walk of the stack's last byte, requiring the flags `map_stack` established:
present, writable, never executable. `top` itself is *not* walked — it is one
past the region, and belongs to whatever is above it or to nothing.

**Rationale.** The caller's question is "may I point the CPU at this now", and
the tables are the only thing that answers it *now*. A root comparison answers
a question from the past and reads like an answer to the present, which is the
same confusion D-94 named: restating a property in a new place is not the same
as holding it.

The cost is one table walk, at boot, once per stack. Option 1 is better and
this does not foreclose it — it makes the value's answer correct while the
value is still a value, which is what a later linear-typed version would also
need for its `is_installed`.

Why not option 2: a generation makes the answer correct and *useless*. The
double-fault stack is installed once at boot and asked about for the life of
the kernel; the first thread to exit would invalidate it. An answer that is
conservative in the direction of refusing the machine's fault handler is not
conservative.

**Consequences.**

- `is_installed` walks. `set_double_fault_stack` and `set_privilege_stack` are
  its callers and both already treat `false` as a refusal, so nothing else
  changed.
- A new machine check, `an unmapped stack is no longer installed`, maps a stack
  above every window the kernel uses, confirms it is installed, unmaps it
  underneath the value, and requires the answer to flip. Broken once against
  the old root-only body and watched go red (D-28).
- The flags are checked, not just presence. A stack that was remapped
  read-only, or executable, is not a stack either, and the walk is already paid
  for.
- D-94's open item is closed. The narrower reading it left — "nothing in the
  tree unmaps a stack, so the window is not reachable" — is now false by
  construction: the check reaches it deliberately.

**Left owed.** The walk is of one page. A stack whose *middle* was unmapped
would still answer `true`, and `map_stack` maps the whole run at once so
nothing produces that state today. Walking every page would make the answer
exact and costs a walk per page at boot — worth doing if a stack ever grows or
shrinks after it is mapped, and not before.
