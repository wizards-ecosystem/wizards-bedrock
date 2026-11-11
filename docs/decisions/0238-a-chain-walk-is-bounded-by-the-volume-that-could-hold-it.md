# D-238 A chain walk is bounded by the volume that could hold it

Status: Accepted, 2026-08-29. Amends D-170 and D-206; advances PW-006.

**Amended by D-352.** The cluster count bounds a chain but can be 268 million,
enough to stall boot. A directory walk now stops at the tighter of the entry
cap and the volume, and a tortoise-and-hare check catches a cycle in its own
length.

**Decision.** The chain bound is the volume's cluster count, computed from the
mounted `Bpb` rather than written down. `chain_limit` is where the pigeonhole
argument lives, and the directory walk, the file read and the skip that precedes
it all use it. A chain that visits more clusters than the volume holds has
visited one twice and is refused as `Unbounded`; a chain that uses every cluster
in the volume still completes.

**Rationale.** `libs/fat32` bounded chains by the fixed `MAX_CHAIN = 1 << 24`
while its comment stated the cluster-count rule. Below sixteen million clusters
a cyclic table drove the walk to that ceiling, so one `OpenAt` on a corrupt
volume could occupy D-200's file worker for tens of millions of sector reads;
above it the constant refused a legitimate chain. `read_file`'s skip loop was
bounded only by the offset ring 3 supplied.

**Alternatives.** Lowering the constant was rejected because any constant is
either above the volume, and useless, or below it, and wrong. Detecting the
cycle exactly, with a visited set or Floyd's algorithm, was rejected because a
set needs allocation this reader does not have and a second cursor doubles the
device reads to learn what the count already proves. Leaving the skip loop
bounded by the caller's offset was rejected because that is ring 3 choosing how
much work the kernel does.

**Consequences.** A sixty-two-cluster volume with a two-cluster cycle refuses
the directory walk and the maximum-offset read within four reads per cluster it
holds. Mutation 285 keeps `a_cyclic_chain_is_refused_within_the_volume` red
with the fixed ceiling restored.
