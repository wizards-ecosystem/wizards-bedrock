# D-207 A foreign FAT corpus is pinned as exact sparse sectors

Status: Accepted, 2026-08-28. Amends D-16 and D-170; closes PW-210.

**Context.** D-170's oracle has two readers but one writer: `fatfs` creates the
volume which both it and `libs/fat32` read. Agreement catches a reader defect,
but a misconception shared with that one formatter can make both readers agree
about bytes no independent formatter would emit. PW-210 makes that proof gap
finite. Pulling `mkfs.fat` or `mtools` into the ordinary build would instead
reopen D-16's host-tool dependency and make CI depend on the host distribution.
A raw valid FAT32 volume is at least tens of MiB even when almost every sector
is zero, so committing it literally would also make the working tree pay for
empty space.

**Decision.** One 64 MiB FAT32 volume is produced outside the build by the
Ubuntu `dosfstools` 4.2-1.2build1 `mkfs.fat` and `mtools` 4.0.49-1 packages.
Their package versions, archive paths, package digests, upstream homes and
licenses are recorded beside the artifact. Neither package nor executable is
vendored. The resulting volume has its own SHA-256 and is committed as a
versioned sparse-sector envelope: the header states the exact expanded length,
sector size and full-image digest, each following record carries one complete
non-zero sector, and every absent sector is exactly zero. The test parser
rejects duplicate, partial, out-of-range and malformed records before
reconstructing the bytes and checking the full-image digest.

The foreign formatter uses non-default reserved geometry and the population
tool leaves valid layouts chosen to defeat the local corpus's conveniences: a
root directory spanning several clusters, a deleted slot before later live
entries, long-name fragments and a file whose cluster chain crosses separated
holes. Host tests read the same reconstructed bytes with `fatfs` and
`libs/fat32`, compare their complete root inventories and file contents, and
independently inspect enough raw layout to prove those cases are present. The
existing locally generated corpus stays: it is the byte-identical image the
kernel boots, while the foreign corpus is an independent host-side format
witness.

**Rationale.** The output bytes, not availability of the generating tools, are
the evidence the reader needs. Pinning both the envelope and expanded image
distinguishes corruption in the compact representation from a change to the
filesystem itself. Whole non-zero sectors keep reconstruction auditable and
need no compression dependency or decoder with its own format surface. A
multi-cluster directory and fragmented file force both kinds of FAT chain walk;
deleted and long-name entries force the two skip cases between live entries.

**Alternatives.** Installing the tools in WSL or requiring them in CI was
rejected because the repository gate must remain self-contained. Generating a
second image with another Rust crate at test time was rejected because that is
still a live dependency rather than pinned independent evidence. Committing
the raw image was rejected because zero-filled sectors convey no additional
proof. Replacing the local corpus was rejected because only it proves that the
host oracle and booted NVMe bytes are identical.

**Consequences.** The 133-line envelope carries 129 non-zero sectors and
reconstructs to exactly 67,108,864 bytes with SHA-256
`06a6ad589c3652731fc60bc669e4e20bc4576da7761c18be0dc9c800fa4d1576`;
the adjacent vendor listing separately pins the envelope itself. Raw host
inspection proves the 64 reserved sectors, two tables, multi-cluster root,
deleted entry before later live entries, volume label, long-name fragments and
40-cluster non-contiguous file are in those bytes rather than merely intended
by the generator.

`fatfs` and `libs/fat32` agree on all 44 root entries, the long file through its
respective long and short names, a nested file and every position-dependent
byte of the fragmented file. Permitting a duplicate envelope sector made the
exact envelope test fail; ending a directory at its first cluster made the
reader lose `FRAG.BIN` and the exact disagreement test fail. Two persistent
host mutations retain those witnesses, bringing the registry to 157. The
ordinary corpus and shipped image are unchanged; release and debug QEMU still
mount and read those exact NVMe bytes and pass all 211 machine checks.
