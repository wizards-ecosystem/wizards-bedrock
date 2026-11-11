# D-73 The filesystem is a destination, not a phase

Status: Provisional, 2026-08-23. Amends D-08.

**Decision.** WizFS leaves Phase 6 for a later phase of its own. Phases 7 to
11 run on a **read-only FAT32 or ext2 driver behind the VFS**. The crash
harness is built before the format is frozen. WizFS's phase has a definition
of done that includes crash injection and a repair tool.

Two corrections to `003`'s rules:

- **Strike the blind-upsert ABI rule.** BetrFS's OS cache answers the
  precondition queries, so upserts run at full speed without an ABI rule;
  the technique is sound and ABI-independent.
- **The reason to skip parity RAID changes.** ZFS's RAID-Z *eliminates* the
  write hole; btrfs's RAID56 and classic RAID5 carry it. The correct reason:
  every correct solution needs buckets, copying GC and backpointers first.

One addition: **bcachefs is not a pure CoW B-tree; its btree updates are
journalled**. That buys prefix consistency, needs no btrfs-style log tree,
and makes small random metadata updates affordable.

**Rationale.** Mature CoW filesystems took a decade or a funded team, and
CoW makes the crash-consistency *argument* simpler and the *implementation*
harder (OSDI '18). Read-only FAT32 tests the NVMe block path, VFS and buffer
plumbing at zero on-disk-write risk; NVMe, because VMware has no virtio
(`010` §10). FAT32's deeper argument is that *the oracle already exists*:
mature operating systems mount the image and judge the writer.

**Consequences.**

- `000-vision.md`'s Phase 6 becomes VFS plus a read-only third-party format;
  WizFS gets a later phase.
- **Free, and do it in v0:** put the `snapshot` field in the key from the
  first format version, always zero. Adding a key field later breaks the
  format; reserving four bytes does not.
- Not in v0: 128 to 256K log-structured nodes, and no copying collector.
- Provisional per D-71: refuted if the crash harness proves unbuildable
  before the format is frozen.
