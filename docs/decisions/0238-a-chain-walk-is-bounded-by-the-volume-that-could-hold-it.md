# D-238 A chain walk is bounded by the volume that could hold it

Status: Accepted (2026-08-29), amends D-170 and D-206; advances PW-006

**Context.** PW-006's hostile-input re-attack reached the chain walking in
`libs/fat32`. Its bound was the fixed `MAX_CHAIN = 1 << 24`, and the comment
directly above the constant stated a different rule: "the bound is the cluster
count, because a chain longer than the volume has visited one twice." The
pigeonhole argument is right; the constant does not implement it, and nothing
noticed because a bound only has to be finite to look correct.

**Amended by D-352.** The cluster count bounds a chain but can be 268 million,
enough to stall boot. A directory walk now stops at the tighter of the entry
cap and the volume, and a tortoise-and-hare check catches a cycle in its own
length.

The gap runs both ways. On a volume smaller than sixteen million clusters — the
shipped 64 MiB image holds about 130,000 — a cyclic table drives the walk to the
fixed ceiling instead of to the volume's own end, and every step is a sector read
a caller waits for. One `OpenAt` on a corrupt volume therefore occupies D-200's
file worker for as long as the device takes to serve tens of millions of reads.
Above sixteen million clusters the same constant refuses a legitimate chain as
`Unbounded`.

`read_file`'s skip loop had no bound at all. It advances `offset / cluster_bytes`
clusters, which the caller's own offset caps near 2^23 for the smallest cluster:
finite, but bounded by the number ring 3 supplied rather than by the volume, and
with no loop detection of its own.

**Decision.** The chain bound is the volume's cluster count, computed from the
mounted `Bpb` rather than written down. `chain_limit` is where the pigeonhole
argument lives, and the directory walk, the file read and the skip that precedes
it all use it. A chain that visits more clusters than the volume holds has
visited one twice and is refused as `Unbounded`; a chain that uses every cluster
in the volume still completes.

**Alternatives.** Lowering the constant was rejected because any constant is the
same defect at a different scale — it is either above the volume, and useless, or
below it, and wrong. Detecting the cycle exactly, with a visited set or Floyd's
algorithm, was rejected because a set needs allocation this reader does not have
and a second cursor doubles the device reads to learn what the count already
proves. Leaving the skip loop bounded by the caller's offset was rejected because
that is ring 3 choosing how much work the kernel does.

**Consequences.** A sixty-two-cluster volume with a two-cluster cycle now refuses
the directory walk and the maximum-offset read within four reads per cluster it
holds; before the change both ran to the counting source's ceiling and returned
`Unreadable`, which is the walk leaving its leash rather than finding the loop.
Restoring the fixed ceiling fails only `a_cyclic_chain_is_refused_within_the_volume`
and mutation 285 retains it. The existing maximum-offset check is unaffected: its
volume holds thirty-three million clusters and its skip is 65,535. No unsafe
operation, wire ABI or machine-check count changed.
