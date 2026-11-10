# D-08 Filesystem: custom WizFS, CoW from the start

Status: Provisional, 2026-07-18. Provisional since 2026-08-23: see D-73, which moves it out of Phase 6.

**Context.** Isaac chose a custom filesystem over adopting ext2, at the
"straight to custom FS" level, then raised the bar to 2026 standards.

**Options.** ext2 clone first; simple extent FS v1 then CoW v2; CoW
checksummed design from the start.

**Decision.** WizFS is copy-on-write, checksummed and snapshotting,
designed once (no throwaway v1), behind a VFS with initrd/tmpfs/devfs as
stepping stones. Design rules locked by the research pass
(bcachefs/btrfs/ZFS/BetrFS evidence, unverified tier, primary-source
backed):

- snapshots as **key versioning**: keys are (inode, offset, snapshot_id);
  O(1) snapshot creation, no tree cloning (bcachefs design; avoids btrfs's
  scalability pain)
- **checksums stored in the btree beside pointers**, never co-located with
  data, because co-located checksums cannot detect lost writes
- **journaled btree updates** in one totally-ordered journal, so prefix
  consistency after crash; large log-structured nodes with our own cache
- **blind upserts in the native API**: the handle ABI (D-06) has no POSIX
  preconditions, so the write-optimized path needs no read-before-write
  (BetrFS lesson)
- inode-style indirection, not full-path keying (rename/delete cost)
- scrub/repair machinery designed in from the start; no parity RAID ever
  (write hole)

**Risk management.** Phase 13 will put the on-disk format in a host-compiling
`libs/wizfs-format` no-std crate, so the same code the kernel mounts can be
unit-tested and fuzzed on Windows. WizFS must carry POSIX semantics
(permissions, symlinks, timestamps) or the D-10 ports suffer.

**Consequences.** The project's highest-risk subsystem (a wrong custom FS
eats data); the mitigations above are non-negotiable.

**Amended by D-73.** The bullets above preserve the founding decision. The
current plan strikes blind upserts as an ABI rule and moves WizFS from
Phase 6 to Phase 13. It rejects parity RAID because its prerequisites are
out of scope, not because RAID-Z has a write hole.
