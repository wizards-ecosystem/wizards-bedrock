# D-170 FAT32, an oracle, and a block layer that is one trait

Status: Accepted (2026-08-25)

**Context.** `012` §1 chose the format on the **corpus**, not on the code, and
was blunt that the roadmap's premise was wrong: measured against GRUB's own two
readers — one project, one style, both read-only — FAT is 673 lines to ext2's
~772. Thirteen per cent. What decided it is that `xtask` already builds a FAT32
volume through `fatfs`, two volumes built seconds apart hash identically, every
pure-Rust ext2 image builder fails to run on this machine, and the reference
read-only ext2 crate generates its corpus by shelling out to `mkfs.ext2`, which
D-16 forbids.

`014` left two decisions open that this slice must take: **is the block layer a
trait or a concrete NVMe**, and **does a page cache exist in this phase**.

**Decision.**

**The block layer is a trait, and it belongs to the reader.** `libs/fat32`
defines `Sectors`, because that is the parameter the reader needs — not a
speculative kernel-side abstraction over a device this kernel has one of
(D-41). The host backs it with a byte slice and the kernel with the controller,
and the *same reader* runs over both. That is not a convenience: it is what
makes an oracle possible at all.

**The oracle is `fatfs` on the host.** Two independent implementations of one
format over one corpus, and the disagreement between them is the test. `012` §1
called this "a better arrangement than a dependency", and the reason is exact:
a dependency cannot disagree with itself.

**No page cache.** `014` carries it as a deliberate hole and the reasoning
holds: reading a directory means reading the same sectors repeatedly, whether
that is affordable is a *measurement*, and the measurement does not exist. A
cache is a decision for whoever has the numbers. The volume is also mounted per
call rather than kept, for the same reason — a boot sector is one sector and the
volume is not going anywhere.

**Rationale, and the corpus doing the work.**

`012` §1 warned that FAT's traps are differently shaped rather than fewer, and
three of them shaped the checks:

- **A cluster number is biased by two**, so the sector of a cluster is
  `first_data + (cluster - 2) * per_cluster`. Forget it and every file is read
  two clusters late, with no error anywhere.
- **A FAT32 entry is twenty-eight bits.** Compare all thirty-two against the
  end-of-chain marker and a chain that has ended is followed into whatever the
  reserved bits were — which is a cluster number, and reads as data.
- **A directory ends at a name beginning zero; `0xE5` is one deleted entry.**
  Confusing them either loses every file below or resurrects deleted ones.

**Four machine breaks passed at first, and the corpus is why.** The end-to-end
checks could not reach the twenty-eight-bit mask (the corpus's reserved bits are
zero), the file-size bound (the check's buffer was cut to the file's length),
the deleted marker (the corpus had no deletions) or the split first-cluster
field (every cluster fits sixteen bits). All four are caught by host tests over
synthetic entries, which is the right division — but two of them were reachable
on the machine for almost nothing, and a real disk is where a real disk lives:

- The corpus now creates a file and **deletes it**, leaving an `0xE5` slot
  *between* live entries. A reader that stopped there loses everything below,
  and the check counts the survivors.
- The read check now offers **a whole sector of buffer for a file that is not
  one**. A reader bounded by the cluster hands back its previous occupant's
  data, and a buffer cut to the file's length could never see it.

That is D-165 again — a check must reach the code it is about — arriving
through the *corpus* rather than through the set-up.

**Consequences.**

- `libs/fat32`: BPB parsing, the table, directory decoding, chain walking,
  per-call offsets, path resolution. No allocation and no panic on hostile
  input: every length checked against what remains, every cluster against the
  data area's bounds, and every walk bounded — a FAT is a linked list a hostile
  volume can make circular, and following one is a hang.
- Eight host tests in the crate, eight more in `xtask::image::corpus` including
  the oracle, four machine checks. Six breaks, all red; two of them made
  reachable on the machine by improving the corpus.
- **512-byte sectors only**, and a volume that says otherwise is refused rather
  than misread. That is what lets every buffer be a fixed array: supporting 4096
  would need a caller-supplied buffer everywhere or an allocator, and no volume
  this project builds or boots uses one.
- The volume sits at **sector 2048** of the namespace, where a partition table
  would put it, and the reader takes a base rather than assuming zero — a reader
  that could not be given an offset is one nothing can be put in front of.
- Three transcriptions meet: the corpus's names and contents, the kernel's copy
  of them, and the base sector stated in two files. A host test compares all of
  them (D-158).
- **The corpus still has one writer.** `014` budgets for it: until a
  foreign-produced FAT32 image is obtained, a shared misreading of the
  specification would pass both implementations. That is the honest limit of an
  oracle built from one tool.
