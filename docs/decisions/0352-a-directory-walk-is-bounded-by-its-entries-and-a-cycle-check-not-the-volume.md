# D-352 A directory walk is bounded by its entries and a cycle check, not the volume

Status: Accepted (2026-09-29), amends D-238; closes PW-562

**Context.** D-238 bounded every chain walk by the data-cluster count, on the
pigeonhole argument that a longer chain has revisited a cluster. That is finite
but not small: a FAT32 volume holds up to 268 million clusters, so a cyclic or
oversized directory chain could drive that many reads before the walk gave up.
The kernel has one filesystem worker, and a mount reads the root directory at
boot, so a hostile or corrupt volume could stall the machine for the length of
that walk. The file-read path already ran a tortoise-and-hare cycle check, but
the directory walk did not.

**Decision.** A FAT directory holds at most 65,536 entries, so a directory
chain needs at most the clusters that hold them. `Bpb::directory_cluster_limit`
returns that count, capped by the data-cluster count, and the directory walk
uses it in place of the volume bound. The directory cursor carries a hare that
advances two clusters for the walk's one; when the hare meets the next cluster
the chain has a cycle and the walk refuses it. A refused transition leaves the
cursor unchanged. The bound no longer grows with the volume, so a cycle is
caught in its own length and a huge acyclic directory stops at the entry cap.
The file-read path keeps its existing hare.

**Evidence.** On 2026-09-29 a two-cycle FAT on a million-cluster volume was
refused after fewer than four cluster steps, far short of the 4,096-cluster cap
and the million-cluster volume, and the cursor was unchanged by the refusal. A
never-ending linear chain on a volume whose 16 KiB clusters cap a directory at
128 clusters stopped at that cap, not the volume's 3,000-plus clusters. The
cluster-limit arithmetic was checked directly for small and large volumes and
cluster sizes. Mutation 1,534 restores the volume bound and fails the acyclic
cap test; mutation 1,535 removes the cycle check and fails the two-cycle test.
Mutation 116, which guarded the old directory volume bound, is retired: both
that bound and its test are gone, and the entry cap is now covered by 1,534.

**Alternatives.** Keeping only the volume bound was rejected because it is 268
million reads in the worst case. A visited-set of every cluster was rejected
because it needs allocation proportional to the directory in a `no_std` reader;
the hare needs none. Refusing every volume whose cluster count exceeds a fixed
ceiling was rejected because a large volume is legitimate; only a single
directory's traversal needs the tighter bound.
