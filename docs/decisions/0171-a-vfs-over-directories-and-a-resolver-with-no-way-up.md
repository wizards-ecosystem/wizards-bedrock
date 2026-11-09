# D-171 A VFS over directories, and a resolver with no way up

Status: Accepted (2026-08-25)

**Context.** `012` §7 was explicit that Linux's shape is the wrong one to copy:
its five operation tables and the dcache exist to serve a global namespace with
a mount tree and a shared lookup cache, and this kernel has none of the three.
Importing the shape without the requirements is how a VFS becomes premature.

`012` §5 was equally explicit about the resolver, and the reason is a security
property rather than a design preference.

**Decision.**

**The abstraction is over directories.** The delivered surface resolves,
reads and enumerates. Mutation remains reserved for the future write path
rather than appearing as an inert method. This compact surface replaces five
Linux-style operation tables because path resolution is the code that must be
backend-agnostic.

**Enum dispatch while every backend is in-kernel.** Monomorphic,
allocation-free, and exhaustively checked by the compiler when a fourth
arrives. Promotion to dynamic dispatch waits for a backend that must be added
*without editing the enum*, which here means a filesystem living in userspace —
and at that point the abstraction is the protocol and the trait is irrelevant.

**The resolver cannot ascend, and that is a property of its input rather than a
check it performs.** `libs/path-walk` resolves `..` lexically and refuses
underflow, so what reaches the resolver is a list of names that only descends.
There is **no parent pointer anywhere in `vfs.rs`** — not a private one, not an
unused one. That is stronger than `RESOLVE_BENEATH`, which enforces a predicate
on a resolver still *capable* of ascending: here there is no capability for a
predicate to be about.

Fuchsia reached the same answer — *Dot Dot Considered Harmful* — and moved it
client-side. This kernel has no trusted client library, so it goes in the
resolver.

**Two refusals, and they are different things.** An absolute path is
`PathError::Absolute` because **there is no root** for it to be relative to; an
escaping `..` is `PathError::Escapes` because there is no parent. Neither is a
denial. Reporting either as a permission failure would suggest that some other
program could, which is false in a way that would send someone looking for the
program that can.

**Nothing is clamped.** An escape is refused rather than resolved to the
starting directory, a name too long is refused rather than truncated, and a
path deeper than the buffer is refused rather than cut short. Each of those
would silently answer a *different* question than the caller asked — and on a
walk that ends in an open, open a different file.

**Rationale, and the thing that hung the harness.**

The enumeration check's first version resolved each listed name from inside the
enumeration callback. Both operations reach the controller through
`nvme::with_controller`, which holds a leaf guard — so the inner call re-entered
the same lock and spun. The harness reported it as a sixty-second timeout,
which is `lock_leaf`'s rule arriving in the only form a same-lock re-entry can
take: the "no lock under a leaf guard" check watches for a *different* lock, and
a spin on the same one is not something a counter can see.

The check now collects the names and resolves them after the enumeration
returns, which is also the shape a caller must use — and the comment says so,
because the next person to write a VFS operation inside a callback will hit the
same wall.

**Consequences.**

- `libs/path-walk`: splitting, `.` and `..` resolution, name validation, depth
  bounds. Nine host tests and a doctest, six deliberate breaks, all red. One of
  the tests is exhaustive over every three-component path built from `a`, `.`,
  `..` and the empty component, because the property is about *every* input.
- `kernel::vfs`: `Directory`, `File`, `Node`, `resolve`, `read`, `enumerate`.
  Four machine checks, five breaks, all red.
- **The depth bound is on the deepest point reached, not on the answer.**
  `a/b/c/../../d` is two components and passes through three, so a buffer of
  two refuses it. Bounding the intermediate is what keeps the work bounded by
  the buffer rather than by the length of an argument ring 3 wrote.
- **Offsets are per operation**, which is `012` §6 and D-152 arriving at the
  file layer: no seek, no tell, no cursor on anything. A shared cursor is a
  lock on the hot path and the only reason two handles to one file would have
  observable coupling — which in a capability system is a covert channel.
- **One namespace per program and no way to compose two.** `014` carries it as
  a deliberate hole; Plan 9's `bind` and Fuchsia's namespace construction are
  out of scope. A program gets the directory handles it was given.
- Enumeration and traversal are **separate authorities**, and the check
  requires them to agree: an enumeration that returned a name traversal cannot
  find is a directory listing that lies. The rights that gate them arrive in
  slice 9.
