# D-170 FAT32, an oracle, and a block layer that is one trait

Status: Accepted, 2026-08-25.

**Decision.** The first filesystem is FAT32, chosen on the corpus, not on the
code (`012` §1): `xtask` already builds a reproducible FAT32 volume through
`fatfs`, while every pure-Rust ext2 image builder fails to run here and the
reference ext2 crate builds its corpus with `mkfs.ext2`, which D-16 forbids.

**The block layer is a trait, and it belongs to the reader.** `libs/fat32`
defines `Sectors`, the parameter the reader needs, rather than a speculative
kernel-side abstraction over a device this kernel has one of (D-41). The host
backs it with a byte slice and the kernel with the controller, and the same
reader runs over both, which is what makes an oracle possible.

**The oracle is `fatfs` on the host.** Two independent implementations of one
format over one corpus, and their disagreement is the test; a dependency cannot
disagree with itself.

**No page cache.** Whether rereading the same sectors is affordable is a
measurement, and the measurement does not exist; a cache is a decision for
whoever has the numbers. The volume is mounted per call for the same reason.

**Rationale.** FAT's traps are differently shaped rather than fewer (`012` §1),
and the reader must get each right: a cluster number is biased by two, so its
sector is `first_data + (cluster - 2) * per_cluster`; a FAT32 entry is
twenty-eight bits, so comparing all thirty-two follows an ended chain into the
reserved bits; and a directory ends at a name beginning zero while `0xE5` is one
deleted entry.

**Consequences.**

- `libs/fat32` does no allocation and has no panic on hostile input: every
  length checked against what remains, every cluster against the data area's
  bounds, and every walk bounded, because a FAT is a linked list a hostile
  volume can make circular.
- The corpus deletes a file, leaving an `0xE5` slot between live entries, and
  the read check offers a whole sector of buffer for a file that is not one, so
  both properties are reachable on the machine. That is D-165 again, a check
  must reach the code it is about, arriving through the corpus. The
  twenty-eight-bit mask and the split first-cluster field are caught by host
  tests over synthetic entries.
- **512-byte sectors only**; a volume that says otherwise is refused rather than
  misread. That lets every buffer be a fixed array, and no volume this project
  builds or boots uses another size.
- The volume sits at **sector 2048** of the namespace, where a partition table
  would put it, and the reader takes a base rather than assuming zero.
- The corpus's names and contents, the kernel's copy of them, and the base
  sector stated in two files are compared by a host test (D-158).
- **The corpus still has one writer.** Until a foreign-produced FAT32 image is
  obtained, a shared misreading of the specification would pass both
  implementations. That is the honest limit of an oracle built from one tool.
