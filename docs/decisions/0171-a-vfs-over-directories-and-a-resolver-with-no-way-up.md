# D-171 A VFS over directories, and a resolver with no way up

Status: Accepted, 2026-08-25.

**Decision.**

**The abstraction is over directories.** The delivered surface resolves, reads
and enumerates. Mutation remains reserved for the future write path rather than
appearing as an inert method. This compact surface replaces Linux's five
operation tables and dcache, which serve a global namespace with a mount tree
and a shared lookup cache this kernel does not have (`012` §7); path resolution
is the code that must be backend-agnostic.

**Enum dispatch while every backend is in-kernel.** Monomorphic,
allocation-free, and exhaustively checked by the compiler when a fourth
arrives. Promotion to dynamic dispatch waits for a backend that must be added
*without editing the enum*, which here means a filesystem living in userspace,
and at that point the abstraction is the protocol and the trait is irrelevant.

**The resolver cannot ascend, and that is a property of its input rather than a
check it performs** (`012` §5). `libs/path-walk` resolves `..` lexically and
refuses underflow, so what reaches the resolver only descends. There is **no
parent pointer anywhere in `vfs.rs`**. That is stronger than `RESOLVE_BENEATH`,
which enforces a predicate on a resolver still capable of ascending. Fuchsia
reached the same answer (*Dot Dot Considered Harmful*) and moved it client-side;
this kernel has no trusted client library, so it goes in the resolver.

**Two refusals, and neither is a denial.** An absolute path is
`PathError::Absolute` because there is no root for it to be relative to; an
escaping `..` is `PathError::Escapes` because there is no parent. Reporting
either as a permission failure would suggest some other program could.

**Nothing is clamped.** An escape, a name too long and a path deeper than the
buffer are refused, not resolved to the start, truncated or cut short, because
each would answer a different question than the caller asked and, on a walk
that ends in an open, open a different file.

**Rationale.** Importing Linux's shape without its requirements is how a VFS
becomes premature, and an ascent-free input is a security property no predicate
can match.

**Consequences.**

- A VFS operation must not run inside an enumeration callback: both reach the
  controller through `nvme::with_controller`'s leaf guard, and the re-entry on
  the same lock spins where no lock counter can see it. Callers collect names
  and resolve them after enumeration returns, and the code comment says so.
- `libs/path-walk` has a test exhaustive over every three-component path built
  from `a`, `.`, `..` and the empty component, because the property is about
  every input.
- **The depth bound is on the deepest point reached, not on the answer.**
  `a/b/c/../../d` is two components and passes through three, so a buffer of
  two refuses it, keeping the work bounded by the buffer rather than by an
  argument ring 3 wrote.
- **Offsets are per operation**, which is `012` §6 and D-152 arriving at the
  file layer: no seek, no tell, no cursor on anything. A shared cursor is a lock
  on the hot path and the only reason two handles to one file would have
  observable coupling, which in a capability system is a covert channel.
- **One namespace per program and no way to compose two.** Plan 9's `bind` and
  Fuchsia's namespace construction are out of scope. A program gets the
  directory handles it was given.
- Enumeration and traversal are **separate authorities**, and the check requires
  them to agree: an enumeration that returned a name traversal cannot find is a
  directory listing that lies. The rights that gate them arrive in slice 9.
