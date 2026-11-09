# D-290 Machine evidence does not widen shipped hardware APIs

Status: Accepted, 2026-09-05. Amends D-124, D-214 and D-248; advances PW-525 and PW-532.

**Context.** Later machine witnesses left observer functions, constants and
evidence tallies compiled into shipped `wost` and kernel paths. Their only
consumers were self-test code, so they widened the production API and added
writes to hardware and lock hot paths without changing production behavior.

**Decision.** A machine-only observer and every write that exists solely to
feed it compile only with `selftest`. Normal EOI, recovery, reclaim and both
`VERW` operations and the lock count that guards context changes remain in
shipped code. A compiler probe requires the exact absence of thirty-three `wost`
observers from the shipped graph. Source contracts pin 96 dedicated evidence
identifiers beneath exact self-test gates: 79 in the kernel, sixteen
Rust-managed tallies, fields or helpers in `wost`, plus the
assembly-visible `BUFFER_CLEARS` state. Both `VERW` tally increments are behind
the same self-test selector; the routed-GSI evidence parameter and call
argument are absent from the shipped graph. Mapping injection and optional EOI
reclamation also compile away as whole branches, leaving direct shipped
operations. Release-ELF inspection
independently found none of that evidence state. The unsafe inventory remains
377 lines in 149 blocks and 39 declarations, with every affected fingerprint
re-reviewed.

**Alternatives and evidence.** Keeping private release counters was rejected:
an uncallable observer still changes the executable path it claims merely to
measure. Relying on optimization was rejected because the source and public
contract would still be wider. Exposing `SAVED_FRAME_BYTES` without its feature
gate made the exact shipped-graph contract red; that defect remains mutation
1,314. Forcing the assembly selector to one remains mutation 1,315. Ungating a
kernel tally remains mutation 1,316; bypassing each assembly path's guard
remains mutations 1,317 and 1,318. Exposing the free-frame and framebuffer-MMIO
base observers remains mutations 1,319 and 1,320; exposing the owned-segment
reader or unmapped-range user bit remains mutations 1,321 and 1,322. The
slab-evidence call and both routed-GSI argument gates remain mutations
1,323 through 1,325. Restoring either no-op shipped branch remains mutations
1,326 and 1,327. Current API audit, Clippy and machine gates remain
PW-525/PW-532's integrated proof.

**Amended by D-361.** On 2026-10-03 the compiler probe grew to thirty-five
absent `wost` observers, adding `PaintedStack` and its painter. Source
contracts now pin 108 dedicated evidence identifiers, 91 of them in the
kernel, adding the NVMe stack meter's seven statics, its leaf hook, its
measurement type and its finishing function.
