# D-08 Filesystem: custom WizFS, CoW from the start

Status: Provisional, 2026-07-18. Provisional since 2026-08-23: see D-73, which moves it out of Phase 6.

**Decision.** WizFS is copy-on-write, checksummed and snapshotting, designed
once (no throwaway v1), behind a VFS with initrd/tmpfs/devfs as stepping
stones. Design rules, from bcachefs/btrfs/ZFS/BetrFS:

- snapshots as **key versioning**: keys are (inode, offset, snapshot_id);
  O(1) snapshot creation, no tree cloning
- **checksums stored in the btree beside pointers**, never co-located with
  data, because co-located checksums cannot detect lost writes
- **journaled btree updates** in one totally-ordered journal, so prefix
  consistency after crash; large log-structured nodes with our own cache
- **blind upserts in the native API**: the handle ABI (D-06) has no POSIX
  preconditions, so the write-optimized path needs no read-before-write
- inode-style indirection, not full-path keying (rename/delete cost)
- scrub/repair machinery designed in from the start; no parity RAID ever
  (write hole)

Phase 13 will put the on-disk format in a host-compiling `libs/wizfs-format`
no-std crate, so the same code the kernel mounts can be unit-tested and
fuzzed on Windows. WizFS must carry POSIX semantics (permissions, symlinks,
timestamps) or the D-10 ports suffer.

**Consequences.** This is the highest-risk subsystem (a wrong custom FS eats
data); the mitigations above are non-negotiable.

**Amended by D-73.** The bullets above preserve the founding decision. The
current plan strikes blind upserts as an ABI rule and moves WizFS from Phase
6 to Phase 13. It rejects parity RAID because its prerequisites are out of
scope, not because RAID-Z has a write hole.
