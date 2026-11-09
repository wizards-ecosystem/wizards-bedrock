# D-73 The filesystem is a destination, not a phase

Status: Provisional, 2026-08-23. Amends D-08.

**Context.** D-08 chose a CoW B-tree with checksums and snapshots, and `003`
set its design rules from bcachefs, btrfs, ZFS and BetrFS. The 2026-08-23
research confirmed almost every rule against primary sources — and then
established what the schedule costs.

bcachefs: announced "more or less feature complete" in August 2015; the
experimental label came off in **June 2026**. Ten years and ten months, by one
exceptional full-time funded engineer — and in between, all 117,000 lines were
removed from Linux 6.18. btrfs: mainline 2009, on-disk format stable late 2013,
and Red Hat *removed* it from RHEL 8 in 2019. ZFS: about five years and a team
growing past twelve paid senior engineers.

And the claim that CoW is the easier road to crash consistency is half false.
OSDI '18: *"btrfs maintains its metadata… in the form of various copy-on-write
B+ trees. This makes achieving crash consistency tricky, as the updates have to
be propagated to several trees."* Of twenty-six crash-consistency bugs studied,
btrfs accounted for twenty-four. CoW makes the *argument* simpler and the
*implementation* harder, and the difference lands in allocation, multi-tree
update ordering, and ENOSPC.

**Decision.** WizFS stops being Phase 6 and becomes a destination reached in a
phase of its own, later. Phases 7–11 run on a **read-only FAT32 or ext2 driver
behind the VFS**. The crash harness is built before the format is frozen.
WizFS's own phase has a definition of done that includes crash injection and a
repair tool.

Two corrections to `003`'s rules while they are open:

- **Strike the blind-upsert ABI rule.** BetrFS states the precondition problem
  and then dissolves it: *"the OS cache of file and directory information was
  able to answer these queries, enabling file creation etc. to run at the full
  speed of upserts."* The technique is sound and ABI-independent; the rule
  solved a problem its own source measured as already solved.
- **The reason to skip parity RAID changes.** ZFS's RAID-Z *eliminates* the
  write hole; it is btrfs's RAID56 and classic RAID5 that carry it. We
  conflated them. The correct reason is that every correct solution needs
  buckets, copying GC and backpointers first.

And one addition that should have been in the rules from the start: **bcachefs
is not a pure CoW B-tree — its btree updates are journalled**, which is what
buys prefix consistency, removes any need for a btrfs-style log tree, and makes
small random metadata updates affordable.

**Rationale.** The design was never the expensive part. Read-only FAT32 makes
the NVMe block path, the VFS, and the buffer plumbing honest at zero
on-disk-write risk — NVMe supersedes the earlier virtio-blk wording because
VMware, the milestone target, has no virtio (`010` §10). FAT32's deeper argument
is that *the oracle already exists*: mount the image on mature operating
systems and let them judge the writer. For a project whose rule is that every
change lands with the check that proves it, that is categorically stronger than
anything buildable for WizFS in one phase.

For calibration, SerenityOS ships ext2 and FAT with hundreds of contributors;
Redox has been committing to `redoxfs` since 2016.

**Consequences.**

- `000-vision.md`'s Phase 6 becomes VFS plus a read-only third-party format;
  WizFS moves to its own later phase.
- **Free, and do it in v0:** put the `snapshot` field in the key from the
  first format version, always zero. Adding a key field later is a format
  break; reserving four bytes is not.
- Not in v0: 128–256K log-structured nodes, and no copying collector.
- Provisional per D-71: refuted if the crash harness proves unbuildable before
  the format is frozen.
