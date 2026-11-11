# D-125 Evidence has a date, and a root match is not a re-walk

Status: Accepted, 2026-08-24.

**Decision.** `MappedStack::is_installed` keeps the root comparison and adds a
walk of the stack's last byte, requiring the flags `map_stack` established:
present, writable, never executable. `top` itself is *not* walked: it is one
past the region, and belongs to whatever is above it or to nothing. A stack
remapped read-only, or executable, is not a stack either.
`set_double_fault_stack` and `set_privilege_stack` treat `false` as a refusal.

**Rationale.** `MappedStack` is evidence dated at construction: nothing consumes
it on unmap, and the root goes on matching for the life of the space, so a
holder could point `IST1` at an address the tables no longer translate, and a
double fault there is a triple fault. The caller's question is "may I point the
CPU at this now", and the tables are the only thing that answers it *now*; a
root comparison answers a question from the past and reads like an answer to
the present. Consuming the value on unmap is better and not foreclosed, but
`unmap` does not know which pages were a stack. A generation bumped by every
unmap would make the answer correct and *useless*: the first thread to exit
would refuse the machine's fault handler.

**Consequences.**

- The check `an unmapped stack is no longer installed` maps a stack above every
  window the kernel uses, unmaps it underneath the value, and requires the
  answer to flip. "Nothing in the tree unmaps a stack" is no argument: the
  check reaches the window deliberately.
- D-94's open item is closed.

**Left owed.** The walk is of one page. A stack whose *middle* was unmapped
would still answer `true`, and `map_stack` maps the whole run at once so
nothing produces that state today. Walking every page would make the answer
exact and costs a walk per page at boot: worth doing if a stack ever grows or
shrinks after it is mapped, and not before.
