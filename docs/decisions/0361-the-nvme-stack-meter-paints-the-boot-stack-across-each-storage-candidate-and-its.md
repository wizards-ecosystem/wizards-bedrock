# D-361 The NVMe stack meter paints the boot stack across each storage candidate, and its scan answers to the live leaf

Status: Accepted, 2026-10-03. Amends D-120, D-290 and D-326; closes PW-575.

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
   existing byte observer was weighed; it disables interrupts and walks the
   page tables once per byte, and its own debug frames would land inside the
   span it reads. The unsafe budget rose from 380 to 381 lines, the measured
   total in 153 blocks.
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

**Rationale.** D-326 read the stack pointer only at NVMe register accesses
during `nvme::prepare`, so frames never live at an access (`QueuePair::new`,
`finish_preparation`, the poll clock, retirement and the MMIO accessors) stayed
invisible, and its window closed before `iommu::establish`, admission,
`nvme::start`, block 0 and retirement ran on the same boot stack, which D-359's
static model alone covered. D-326 rejected painting because a scan cannot see
space a frame reserves and never writes; that holds for a paint used alone,
and it is why the leaf stays beside the paint.

**Evidence.** On 2026-10-03 the self-tests reported
`nvme-stack v2 used=23792 capacity=32768 headroom=8976 leaf=22256` in debug and
`nvme-stack v2 used=13240 capacity=32768 headroom=19528 leaf=11848` in release.
Mutation 1,590 adds a frame to `finish_preparation`, which makes no register
access, of 8 KiB in debug and 16 KiB in release, and failed only the headroom
check in each profile while the leaf alone left more than two pages free;
1,591 starts the scan at the paint's own boundary and failed the same check;
mutation 17 still fails it in release. Mutation 1,592 ungates `PaintedStack` and fails
`contracts::machine_probes_are_absent_from_the_shipped_wost_graph`, and 1,593
ungates the leaf hook and fails
`contracts::machine_evidence_writes_are_absent_from_shipped_paths`; in
`stack-layout`, 1,594 and 1,595 fail
`usage_splits_a_slot_at_every_pointer_inside_it_and_refuses_the_rest` and
`a_span_below_a_pointer_starts_at_the_base_and_stops_a_clearance_short`; and
1,596, which bounds the leaf by the capacity instead of the painted mark, fails
`qemu::tests::a_passing_selftest_requires_bounded_conservative_nvme_stack_evidence`.

**Alternatives.** Replacing the leaf with the paint was rejected: the leaf is
the evidence that the scan read the words it reports. Taking the larger of the
two readings was rejected because it would hide a scan that read less than the
leaf proves. Painting around `nvme::prepare` alone was rejected: the window
would still close with preparation, and under fat LTO the release image folded
preparation's stages into one frame whose executed callees reached a few
hundred bytes below it, so a paint 1 KiB below its painter would have seen none
of preparation's writes. Keeping preparation's stages out of line in the
self-test image is not needed at the sequencer, where `nvme::prepare` has its
own frame below the paint. Finishing the measurement from a `Drop` implementation was
rejected because the API audit admits implicit drops only for reviewed types,
and explicit finishing leaves that boundary unchanged. A larger boot stack was
rejected for the reasons D-287, D-326 and D-359 give.

**Left owed.** The meter measures the paths QEMU runs. The static model of the
debug image, which sums direct-call prologue frames and does not follow
formatting code reached through trait objects, puts the deepest unexecuted
path, the retirement of a controller whose start fails, at 24,256 bytes, which
would leave 8,512 free.
