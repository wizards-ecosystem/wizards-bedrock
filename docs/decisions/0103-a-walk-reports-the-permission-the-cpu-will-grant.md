# D-103 A walk reports the permission the CPU will grant

Status: Accepted, 2026-08-24.

**Context.** `paging::translate` returned the *leaf* entry's flags. The
architecture does not: `USER` and `WRITABLE` are the conjunction down the whole
walk and `NO_EXECUTE` the disjunction. A leaf that says `USER` beneath a parent
that does not is not user memory. Four callers read the result as effective,
`enter_user`'s two checks and `admits`' two, and each documents that stronger
property.

D-101's audit found it and correctly ranked it benign. It holds today only
because `Entry::table` always sets `PRESENT | WRITABLE` and never `NO_EXECUTE`,
and because `set` upgrades parent entries to `USER` when a user leaf needs one.
The single narrowable bit is repaired on the way down.

**Decision.** The walk narrows as it goes. `USER` and `WRITABLE` are intersected
at every level; `NO_EXECUTE` is accumulated. The leaf's other bits
(cacheability, the owned bit, `PRESENT`) are its own.

**Rationale.** A property that holds because of two unrelated invariants
elsewhere breaks when someone changes one of them. A second address space with a
read-only shared region would map a parent read-only or non-executable. Then
`admits` would pass and `copy_to_user` would take a terminal fault, the failure
D-97 exists to avoid.

Computing it correctly costs one `and` and one `or` per level of a walk that
already reads every level.

**Consequences.**

- One hundred and four named machine checks. The new one is at the *walk* level,
  not in `libs/x86-paging`, because the property needs a table hierarchy where a
  parent disagrees with a child. Only a live address space has one.
- Breaking the narrowing turns **forty** checks red, not one. Every guard-page
  check, every mapping check and every ring-3 check reads a permission through
  this walk.
- `Flags::intersection` joins `libs/x86-paging`, host-tested. An operation
  returning the union, or either operand, would make a walk report a permission
  the CPU does not grant.
