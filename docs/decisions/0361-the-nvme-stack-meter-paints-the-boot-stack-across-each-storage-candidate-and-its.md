# D-361 The NVMe stack meter paints the boot stack across each storage candidate, and its scan answers to the live leaf

Status: Accepted, 2026-10-03. Amends D-120, D-290 and D-326; closes PW-575.

**Context.** D-326 reads the stack pointer at every NVMe register access while
`nvme::prepare` runs. A frame that is never live at an access stays invisible
to it: the allocation and zeroing in `QueuePair::new`, `finish_preparation`,
the poll clock, the retirement paths, and the MMIO accessor frames beneath the
access itself. Its window also closed when preparation returned, although
`iommu::establish`, bus-master admission, `nvme::start`, the block 0 read and
retirement run on the same guarded 32 KiB boot stack. D-359 sequenced the
candidate through sibling stages so that a static model of the debug image
left at least 8,560 bytes free on every path, and no machine check measured
those paths. D-326 rejected painting because a scan cannot see space a frame
reserves and never writes. That holds for a paint used alone, and it is why
the leaf stays beside the paint.

**Decision.**

1. *`wost` paints and scans.* `paging::paint_current_permanent_stack_for_test`
   finds the permanent stack in the live root that holds the stack pointer,
   re-walks its guard and pages, and fills every word from the stack's base to
   1 KiB below its own frame with a word that is non-zero, because a stack
   probe stores zero, and non-canonical, so that no saved pointer equals it.
   `PaintedStack::deepest_touch` refuses unless it runs on that stack in the
   same root with half a kilobyte between its frame and the painted span, and
   then returns the lowest word that no longer holds the paint. Both exist only
   in the self-test image. The span arithmetic is
   `stack_layout::Slot::span_below`, and host properties check it and
   `Slot::usage_at` for every accepted layout and every placement of the
   pointer.
2. *Two lines of new `unsafe`.* The paint's volatile store and the scan's
   volatile load are one-line blocks under D-41's first reason: no safe door
   writes or reads the dead part of a live stack. A scan built from the
   existing byte observer was weighed. It disables interrupts and walks the
   page tables once per byte, and its own debug frames would land inside the
   span it reads. The unsafe budget rises from 380 to 381 lines, the measured
   total in 153 blocks, and the reviewed fingerprint of `paging.rs` records
   thirteen blocks.
3. *The window is the whole candidate.* `try_controller` begins a
   `nvme::CandidateStackMeasurement` as its first statement and finishes it on
   both paths it returns by, so the claim, isolation, admission, start, block 0
   and retirement stages fall inside one window, and the register-access leaf
   observes the same window. The meter no longer wraps `nvme::prepare`, and the
   wrapper that existed for it merged into `prepare`. The finishing calls take
   the measurement by reference, because a by-value move copies it into the
   debug sequencer's frame at each call.
4. *The leaf is the scan's oracle.* The word at every observed leaf pointer
   was written by the call that observed it, so a painted mark shallower than
   the deepest leaf is a scan that missed a write, and that candidate's
   measurement is refused. Otherwise the painted mark is the candidate's use,
   and the deepest across candidates must leave two pages of the 32 KiB boot
   stack free in both profiles. The check is renamed
   `nvme candidates preserve two pages of boot-stack headroom` and emits
   `nvme-stack v2 used=N capacity=N headroom=N leaf=N`. The host parser refuses
   v1, a missing, reordered or non-canonical field, and a leaf deeper than the
   painted mark.
5. *D-290's inventories grow.* The compiler probe also requires
   `PaintedStack` and its painter to be absent from the shipped graph,
   thirty-five observers in all. The source contract pins the meter's seven
   statics, its leaf hook, its measurement type and its finishing function in
   `nvme.rs`, and the measurement type and both finishing calls in `main.rs`.

**Evidence.** On 2026-10-03 the debug self-test passed 271/271 with
`nvme-stack v2 used=23792 capacity=32768 headroom=8976 leaf=22256`, and the
release self-test passed 271/271 with
`nvme-stack v2 used=13240 capacity=32768 headroom=19528 leaf=11848`. The debug
sequencer frame grew from 2,592 to 2,704 bytes and preparation lost its
256-byte wrapper frame; the release sequencer frame grew from 5,136 to 5,232
bytes. In the release image the painter is inlined into the sequencer's frame
and `nvme::prepare` keeps a 1,648-byte frame of its own beneath it, deeper than
the paint's 1 KiB clearance, so preparation's register-access leaves fall
inside the painted span. Mutation 1,590 adds a frame to `finish_preparation`,
which makes no register access: 8 KiB in debug and 16 KiB in release. Each
self-test then reported `verdict v1 FAIL passed=270 failed=1 expected=271`
with only `nvme candidates preserve two pages of boot-stack headroom` failed, at
`used=26056 capacity=32768 headroom=6712 leaf=22256` in debug and
`used=28296 capacity=32768 headroom=4472 leaf=11896` in release, and the leaf
of either record alone leaves more than two pages free. Mutation 1,591 starts
the scan at the paint's own boundary; both profiles failed the same sole check
with no record printed. Mutation 17 still fails it in release, at
`used=28248 capacity=32768 headroom=4520 leaf=28248`. Mutation 1,592 removes
the self-test gate from `PaintedStack`, and
`contracts::machine_probes_are_absent_from_the_shipped_wost_graph` failed with
`a machine-only observer reached shipped code`. Mutation 1,593 ungates the leaf
hook in `read`, and `contracts::machine_evidence_writes_are_absent_from_shipped_paths`
failed with
`kernel/src/nvme.rs has shipped machine-observer uses: {"observe_candidate_stack": 1}`.
In `stack-layout`, mutation 1,594 fixes the capacity at 16 KiB and failed
`usage_splits_a_slot_at_every_pointer_inside_it_and_refuses_the_rest` with
`capacity is not the slot's length` while all ten unit tests passed, and
mutation 1,595 lengthens the span by a word and failed
`a_span_below_a_pointer_starts_at_the_base_and_stops_a_clearance_short` with
`the span reaches into the clearance`. Mutation 1,596 bounds the leaf by the
capacity instead of the painted mark, and
`qemu::tests::a_passing_selftest_requires_bounded_conservative_nvme_stack_evidence`
failed when a record with the deeper leaf passed.

**Alternatives.** Replacing the leaf with the paint was rejected: the leaf is
the evidence that the scan read the words it reports. Taking the larger of the
two readings was rejected because it would hide a scan that read less than the
leaf proves. Painting around `nvme::prepare` alone, as the first plan for this
decision proposed, was rejected: the window would still close with preparation,
and under fat LTO the release image folded preparation's stages into one frame
whose executed callees reached a few hundred bytes below it, so a paint 1 KiB
below its painter would have seen none of preparation's writes. That plan's
remedy, keeping preparation's stages out of line in the self-test image, is not
needed at the sequencer, where `nvme::prepare` has its own frame below the
paint. Finishing the measurement from a `Drop` implementation was rejected
because the API audit admits implicit drops only for reviewed types, and
explicit finishing leaves that boundary unchanged. A larger boot stack was
rejected for the reasons D-287, D-326 and D-359 give.

**Left owed.** The meter measures the paths QEMU runs. The static model of the
debug image, which sums direct-call prologue frames and does not follow
formatting code reached through trait objects, puts the deepest unexecuted
path, the retirement of a controller whose start fails, at 24,256 bytes, which
would leave 8,512 free.
