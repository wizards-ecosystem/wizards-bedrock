# D-73 The filesystem is a destination, not a phase

Status: Provisional, 2026-08-23. Amends D-08.

**Context.** D-08 chose a CoW B-tree with checksums and snapshots; `003` set
its rules from bcachefs, btrfs, ZFS and BetrFS. The 2026-08-23 research
confirmed almost every rule against primary sources, then priced the schedule.

bcachefs: announced "more or less feature complete" in August 2015; the
experimental label came off in **June 2026**. Ten years and ten months, by one
exceptional full-time funded engineer, and meanwhile all 117,000 lines left
Linux 6.18. btrfs: mainline 2009, on-disk format stable late 2013, and Red Hat
*removed* it from RHEL 8 in 2019. ZFS: about five years and a team growing
past twelve paid senior engineers.

That CoW is the easier road to crash consistency is half false. OSDI '18:
*"btrfs maintains its metadata… in the form of various copy-on-write B+ trees.
This makes achieving crash consistency tricky, as the updates have to be
propagated to several trees."* Of twenty-six crash-consistency bugs studied,
btrfs accounted for twenty-four. CoW makes the *argument* simpler and the
*implementation* harder, in allocation, multi-tree update ordering and ENOSPC.

**Decision.** WizFS leaves Phase 6 for a later phase of its own. Phases 7 to
11 run on a **read-only FAT32 or ext2 driver behind the VFS**. The crash
harness is built before the format is frozen. WizFS's phase has a definition
of done that includes crash injection and a repair tool.

Two corrections to `003`'s rules while they are open:

- **Strike the blind-upsert ABI rule.** BetrFS states the precondition
  problem, then dissolves it: *"the OS cache of file and directory information
  was able to answer these queries, enabling file creation etc. to run at the
  full speed of upserts."* The technique is sound and ABI-independent; the
  rule solved a problem its source measured as solved.
- **The reason to skip parity RAID changes.** ZFS's RAID-Z *eliminates* the
  write hole; btrfs's RAID56 and classic RAID5 carry it, and we conflated
  them. The correct reason: every correct solution needs buckets, copying GC
  and backpointers first.

One addition was owed from the start: **bcachefs is not a pure CoW B-tree; its
btree updates are journalled**. That buys prefix consistency, needs no
btrfs-style log tree, and makes small random metadata updates affordable.

**Rationale.** The design was never the cost. Read-only FAT32 tests the NVMe
block path, VFS and buffer plumbing at zero on-disk-write risk. NVMe replaces
the earlier virtio-blk wording because VMware, the milestone target, has no
virtio (`010` §10). FAT32's deeper argument is that *the oracle already exists*:
mature operating systems mount the image and judge the writer. For a project
whose rule is that every change lands with the check that proves it, that is
categorically stronger than anything buildable for WizFS in one phase.

For calibration, SerenityOS ships ext2 and FAT with hundreds of contributors;
Redox has been committing to `redoxfs` since 2016.

**Consequences.**

- `000-vision.md`'s Phase 6 becomes VFS plus a read-only third-party format;
  WizFS gets a later phase.
- **Free, and do it in v0:** put the `snapshot` field in the key from the
  first format version, always zero. Adding a key field later breaks the
  format; reserving four bytes does not.
- Not in v0: 128 to 256K log-structured nodes, and no copying collector.
- Provisional per D-71: refuted if the crash harness proves unbuildable before
  the format is frozen.
