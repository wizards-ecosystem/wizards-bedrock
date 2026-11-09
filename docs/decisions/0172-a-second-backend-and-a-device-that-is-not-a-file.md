# D-172 A second backend, and a device that is not a file

Status: Accepted, 2026-08-25.

**Context.** `014` is blunt about why this slice exists: *the VFS needs at least
two backends before its abstraction is anything but a guess.* A directory trait
with one implementation is a description of that implementation, and slice 7's
three operations had only FAT32 to be about.

`012` §7 is equally blunt about the second thing: **devfs does not go through a
byte interface.** A devfs entry has no bytes; it is a name that resolves to a
device object *of another type*. Forcing it to implement `read_at` is the
"everything is a file" conflation, and WASI's own README rejects it on exactly
these grounds.

**Decision.** Two more backends, chosen to be unlike the first in different
directions.

**tmpfs is unlike FAT32 in the ways that matter.** No clusters, no chains, no
device; its directories are a flat table with parent indices rather than a
linked structure on a disk. If `lookup`, `enumerate` and `read` fit both, they
are about directories rather than about FAT — which is the whole claim.

**devfs is unlike both in the way that matters most.** `Node` gains a third
variant, and `vfs::read` takes a `File`. **There is no way to pass a device to
it** — not a refusal at run time, an absence at compile time. That is the
difference between a design that says devices are not files and one that
enforces it.

**Fixed-size and read-only.** The tmpfs arena is a `static`, because the write
path is this phase's largest deliberate hole and a tmpfs that could grow would
be the write path arriving through the back door.

**Rationale, and the corpus catching the check again.**

Adding a `Node` variant was a compile error in every match that handles one,
which is exactly what the enum-dispatch decision (D-171) was for: a fourth
backend is a build failure rather than a number that means nothing. Promotion
to dynamic dispatch still waits for a backend that must be added *without*
editing the enum.

**A tmpfs lookup that ignored the parent passed.** Every name in the first tree
was unique — `MOTD`, `NOTES`, `ONE.TXT` — so a lookup that searched the whole
arena found the right entry anyway, and deleting the containment check left the
boot green. The tree now holds `MOTD` **twice**, once at the root and once
inside `NOTES` with different content, and the check reads both. That is D-155's
rule for the third time in this phase, and the third different place it has
arrived from: an input, a corpus, and now a directory tree.

**Consequences.**

- `kernel::tmpfs` and `kernel::devfs`; `vfs::Directory`, `File` and `Node` each
  gain the variants they need. Three machine checks, five breaks, all red.
- The no-ascent invariant is checked on **every backend**, not on FAT — it is a
  property of the resolver, and a check that only exercised one backend would
  be a property of that backend.
- **A directory handle names its own backend.** A tmpfs handle cannot resolve a
  name that exists only on the volume, and the check requires each name to
  resolve where it belongs *and* nowhere else — so the refusals are about the
  backend rather than about the name.
- devfs is **one directory, not a tree**: `storage/anything` is refused for the
  same reason a path through a file is. A devfs with subdirectories would be a
  namespace nobody asked for.
- `Device` is an enum rather than an index into a registry, so a fourth device
  is a compile error everywhere that matches on it. There is one driver and a
  registry with one entry is a description of that entry (D-41).
- What a program will eventually receive from devfs is slice 4's `Device`
  capability. Today the name resolves; the handle it becomes is Phase 7's, and
  saying so is better than building a table nothing puts anything in.
