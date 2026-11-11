# D-352 A directory walk is bounded by its entries and a cycle check, not the volume

Status: Accepted, 2026-09-29. Amends D-238; closes PW-562.

**Decision.** A FAT directory holds at most 65,536 entries, so a directory
chain needs at most the clusters that hold them. `Bpb::directory_cluster_limit`
returns that count, capped by the data-cluster count, and the directory walk
uses it in place of the volume bound. The directory cursor carries a hare that
advances two clusters for the walk's one; when the hare meets the next cluster
the chain has a cycle and the walk refuses it. A refused transition leaves the
cursor unchanged. The bound no longer grows with the volume, so a cycle is
caught in its own length and a huge acyclic directory stops at the entry cap.
The file-read path keeps its existing hare.

**Rationale.** D-238's data-cluster bound is finite but can be 268 million
clusters on FAT32, and with one filesystem worker and a root directory read at
mount, a hostile or corrupt volume could stall boot for that walk. Mutation
1,534 restores the volume bound and fails the acyclic cap test; mutation 1,535
removes the cycle check and fails the two-cycle test. Mutation 116, which
guarded the old directory volume bound, is retired with that bound and its
test; the entry cap is covered by 1,534.

**Alternatives.** Keeping only the volume bound was rejected because it is 268
million reads in the worst case. A visited-set of every cluster was rejected
because it needs allocation proportional to the directory in a `no_std` reader;
the hare needs none. Refusing every volume whose cluster count exceeds a fixed
ceiling was rejected because a large volume is legitimate; only a single
directory's traversal needs the tighter bound.
