# D-103 A walk reports the permission the CPU will grant

Status: Accepted, 2026-08-24.

**Decision.** `paging::translate` narrows as it walks. `USER` and `WRITABLE`
are intersected at every level; `NO_EXECUTE` is accumulated. The leaf's other
bits (cacheability, the owned bit, `PRESENT`) are its own. This is what the
architecture grants, and what `enter_user`'s and `admits`' callers already
documented.

**Rationale.** The leaf-only answer held only because `Entry::table` never
narrows a parent and `set` upgrades parents to `USER`; a property that holds
because of two unrelated invariants elsewhere breaks when someone changes one
of them. A read-only or non-executable parent would then let `admits` pass and
`copy_to_user` take a terminal fault, the failure D-97 exists to avoid.
Computing it correctly costs one `and` and one `or` per level.

**Consequences.**

- The check is at the *walk* level, not in `libs/x86-paging`, because the
  property needs a table hierarchy where a parent disagrees with a child. Only
  a live address space has one.
- `Flags::intersection` joins `libs/x86-paging`, host-tested. An operation
  returning the union, or either operand, would make a walk report a permission
  the CPU does not grant.
