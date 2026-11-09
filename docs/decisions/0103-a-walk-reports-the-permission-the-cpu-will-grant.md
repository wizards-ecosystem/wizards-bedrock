# D-103 A walk reports the permission the CPU will grant

Status: Accepted, 2026-08-24.

**Context.** `paging::translate` returned the *leaf* entry's flags. The
architecture does not: `USER` and `WRITABLE` are the conjunction down the whole
walk, and `NO_EXECUTE` is the disjunction, so a leaf that says `USER` beneath a
parent that does not is not user memory. Four callers read the result as if it
were effective — `enter_user`'s two checks and `admits`' two — and each states
that stronger property in its own documentation.

D-101's audit found it and ranked it benign, correctly: it holds today only
because `Entry::table` unconditionally sets `PRESENT | WRITABLE` and never
`NO_EXECUTE`, and because `set` upgrades parent entries to `USER` when a user
leaf needs one. The single narrowable bit is repaired on the way down.

**Decision.** The walk narrows as it goes. `USER` and `WRITABLE` are
intersected at every level; `NO_EXECUTE` is accumulated. The leaf's other bits
— cacheability, the owned bit, `PRESENT` — are its own.

**Rationale.** "It holds because of two unrelated invariants elsewhere" is the
shape of a bug waiting for someone to change one of them. The day a parent is
mapped read-only or non-executable — which a second address space with a
read-only shared region would want — `admits` would pass and `copy_to_user`
would take a terminal fault, which is the exact failure D-97 exists to avoid.

Computing it correctly costs one `and` and one `or` per level of a walk that
already reads every level.

**Consequences.**

- One hundred and four named machine checks. The new one is at the *walk*
  level rather than in `libs/x86-paging`, because the property needs a table
  hierarchy where a parent disagrees with a child, and only a real address
  space has one.
- Breaking the narrowing turns **forty** checks red, not one. That is the
  clearest evidence available that this is load-bearing rather than
  theoretical: every guard-page check, every mapping check and every ring-3
  check reads a permission through this walk.
- `Flags::intersection` joins `libs/x86-paging`, host-tested — an operation
  that returned the union, or either operand, would make a walk report a
  permission the CPU does not grant.
