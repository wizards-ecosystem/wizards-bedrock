# D-172 A second backend, and a device that is not a file

Status: Accepted, 2026-08-25.

**Decision.** The VFS gets two more backends, each unlike FAT32 in a different
direction, because a directory trait with one implementation is a description
of that implementation.

**tmpfs is unlike FAT32 in the ways that matter.** No clusters, no chains, no
device; its directories are a flat table with parent indices. If `lookup`,
`enumerate` and `read` fit both, they are about directories rather than about
FAT. Its arena is a fixed-size, read-only `static`, because a tmpfs that could
grow would be the write path, this phase's largest deliberate hole, arriving
through the back door.

**devfs does not go through a byte interface** (`012` §7). A devfs entry is a
name that resolves to a device object of another type; forcing it to implement
`read_at` is the "everything is a file" conflation WASI's own README rejects.
`Node` gains a third variant, and `vfs::read` takes a `File`, so **there is no
way to pass a device to it**: not a refusal at run time, an absence at compile
time.

**Rationale.** Adding a `Node` variant was a compile error in every match that
handles one, which is what the enum-dispatch decision (D-171) was for; promotion
to dynamic dispatch still waits for a backend that must be added without editing
the enum.

**Consequences.**

- The tmpfs tree holds `MOTD` twice, at the root and inside `NOTES` with
  different content, and the check reads both, so a lookup that ignored the
  parent fails. That is D-155's rule arriving from a directory tree.
- The no-ascent invariant is checked on **every backend**, because it is a
  property of the resolver, not of one backend.
- **A directory handle names its own backend.** The check requires each name to
  resolve where it belongs *and* nowhere else.
- devfs is **one directory, not a tree**: `storage/anything` is refused for the
  same reason a path through a file is.
- `Device` is an enum rather than an index into a registry, so a fourth device
  is a compile error everywhere that matches on it. There is one driver, and a
  registry with one entry is a description of that entry (D-41).
- What a program will eventually receive from devfs is slice 4's `Device`
  capability. Today the name resolves; the handle it becomes is Phase 7's,
  rather than a table nothing puts anything in.
